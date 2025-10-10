use std::sync::{Arc, LazyLock};

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
};
use color_eyre::{
    Result,
    eyre::{Context, OptionExt},
};
use derive_more::{Debug, Display, From, Into};
use hashlru::SyncCache;
use rand::Rng;
use rusty_paseto::prelude::{
    AudienceClaim, CustomClaim, IssuerClaim, Key as PasetoKey, Local as PasetoLocal, PasetoBuilder,
    PasetoParser, PasetoSymmetricKey, SubjectClaim, V4 as PasetoV4,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tap::Pipe;

use crate::storage::Key;

/// The secret key for stateless tokens.
///
/// We generate this at startup because the stateless tokens are only relevant
/// for a single running API instance; they're not valid across multiple
/// instances or restarted instances.
///
/// Each instance having its own secret is also nice in that we don't have to
/// worry about manually validating that the `org_id` passed to the request via
/// header and the `org_id` in the token match: the stateless token will just
/// fail to parse if the user provided a different `org_id` in the header
/// (assuming that caused the ingress to route the request to the wrong
/// backend instance).
static STATELESS_TOKEN_SECRET: LazyLock<PasetoSymmetricKey<PasetoV4, PasetoLocal>> =
    LazyLock::new(|| {
        let mut key = [0u8; 32];
        rand::rng().fill(&mut key[..]);
        PasetoSymmetricKey::from(PasetoKey::from(key))
    });

/// Stateless token providing pre-authorized org and account IDs, plus the original
/// token used to mint the stateless token (intended to support interacting with
/// the database).
///
/// This type is technically equivalent to [`AuthenticatedToken`], but has
/// different semantics; due to this it is possible to freely convert between
/// the two types without any loss of information or even cloning.
#[derive(Clone, Debug)]
#[debug("StatelessToken(org_id = {}, account_id = {})", org_id, account_id)]
pub struct StatelessToken {
    /// The authenticated organization ID.
    pub org_id: OrgId,

    /// The authenticated account ID.
    pub account_id: AccountId,

    /// The original token used to mint the stateless token.
    pub token: RawToken,
}

impl StatelessToken {
    const CLAIM_AUDIENCE: &str = "hurry";
    const CLAIM_SUBJECT: &str = "cas";
    const CLAIM_ISSUER: &str = "courier";
    const CLAIM_ORG_ID: &str = "x-org-id";
    const CLAIM_ACCOUNT_ID: &str = "x-account-id";
    const CLAIM_TOKEN: &str = "x-token";

    fn audience() -> AudienceClaim<'static> {
        AudienceClaim::from(Self::CLAIM_AUDIENCE)
    }

    fn subject() -> SubjectClaim<'static> {
        SubjectClaim::from(Self::CLAIM_SUBJECT)
    }

    fn issuer() -> IssuerClaim<'static> {
        IssuerClaim::from(Self::CLAIM_ISSUER)
    }

    fn org_id(&self) -> Result<CustomClaim<u64>> {
        CustomClaim::try_from((Self::CLAIM_ORG_ID, self.org_id.as_u64()))
            .context("custom claim org id")
    }

    fn account_id(&self) -> Result<CustomClaim<u64>> {
        CustomClaim::try_from((Self::CLAIM_ACCOUNT_ID, self.account_id.as_u64()))
            .context("custom claim account id")
    }

    fn token(&self) -> Result<CustomClaim<String>> {
        CustomClaim::try_from((Self::CLAIM_TOKEN, self.token.0.clone()))
            .context("custom claim token")
    }

    /// Serialize the stateless token to a string.
    pub fn serialize(&self) -> Result<String> {
        let org_id = Self::org_id(self)?;
        let account_id = Self::account_id(self)?;
        let token = Self::token(self)?;
        PasetoBuilder::<PasetoV4, PasetoLocal>::default()
            .set_claim(Self::audience())
            .set_claim(Self::subject())
            .set_claim(Self::issuer())
            .set_claim(org_id)
            .set_claim(account_id)
            .set_claim(token)
            .build(&STATELESS_TOKEN_SECRET)
            .context("build token")
    }

    /// Deserialize a stateless token from a string.
    pub fn deserialize(token: &str) -> Result<Self> {
        let parsed = PasetoParser::<PasetoV4, PasetoLocal>::default()
            .check_claim(Self::audience())
            .check_claim(Self::subject())
            .check_claim(Self::issuer())
            .parse(token, &STATELESS_TOKEN_SECRET)
            .context("parse token")?;

        let org_id = parsed[Self::CLAIM_ORG_ID]
            .as_u64()
            .ok_or_eyre("no org id")?;
        let account_id = parsed[Self::CLAIM_ACCOUNT_ID]
            .as_u64()
            .ok_or_eyre("no account id")?;
        let token = parsed[Self::CLAIM_TOKEN]
            .as_str()
            .ok_or_eyre("no raw token")?;

        Ok(Self {
            org_id: OrgId::from_u64(org_id),
            account_id: AccountId::from_u64(account_id),
            token: RawToken::new(token),
        })
    }
}

impl Serialize for StatelessToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let token = self.serialize().map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(&token)
    }
}

impl<'de> Deserialize<'de> for StatelessToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let token = String::deserialize(deserializer)?;
        Self::deserialize(&token).map_err(serde::de::Error::custom)
    }
}

impl From<StatelessToken> for AuthenticatedToken {
    fn from(jwt: StatelessToken) -> Self {
        Self {
            account_id: jwt.account_id,
            org_id: jwt.org_id,
            token: jwt.token,
        }
    }
}

impl From<&StatelessToken> for AuthenticatedToken {
    fn from(jwt: &StatelessToken) -> Self {
        jwt.clone().into()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for StatelessToken {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let Some(header) = parts.headers.get(AUTHORIZATION) else {
            return Err((
                StatusCode::UNAUTHORIZED,
                String::from("Authorization header required"),
            ));
        };
        let Ok(token) = header.to_str() else {
            return Err((
                StatusCode::BAD_REQUEST,
                String::from("Authorization header must be a string"),
            ));
        };

        let token = match token.strip_prefix("Bearer") {
            Some(token) => token.trim(),
            None => token.trim(),
        };
        if token.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                String::from("Empty authorization token"),
            ));
        }

        StatelessToken::deserialize(token).map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))
    }
}

/// In-memory collection of [`OrgKeySet`] per organization.
///
/// This type uses interior mutability to allow for thread-safe operations; even
/// when you clone an instance the internal cache is still shared with the
/// original instance.
///
/// This type implements an LRU cache per organization; by default the set holds
/// up to [`KeySets::DEFAULT_LIMIT`] organizations. Each `OrgKeySet` that's
/// created is initialized with an LRU cache that holds up to
/// [`OrgKeySet::DEFAULT_LIMIT`] keys by default.
#[derive(Clone, Debug)]
#[debug("KeySets(count = {})", self.0.len())]
pub struct KeySets(Arc<SyncCache<OrgId, OrgKeySet>>);

impl KeySets {
    /// The default number of organizations to cache.
    ///
    /// If more than this number of keys are inserted into the set, the least
    /// recently used key is evicted.
    pub const DEFAULT_LIMIT: u64 = 100;

    /// Create a new instance with the default limit.
    pub fn new() -> Self {
        SyncCache::new(Self::DEFAULT_LIMIT as usize)
            .pipe(Arc::new)
            .pipe(Self)
    }

    /// Get the set of allowed CAS keys for the given organization.
    ///
    /// If the set for the organization is not in the cache, a new set is
    /// created and inserted, then returned.
    ///
    /// Reminder that [`OrgKeySet`] uses interior mutability to allow clones to
    /// share the same underlying data; even though you're getting an owned
    /// instance changes to this instance will be reflected in all clones.
    pub fn organization(&self, id: OrgId) -> OrgKeySet {
        match self.0.get(&id) {
            Some(set) => set,
            None => {
                let set = OrgKeySet::new();
                self.0.insert(id, set.clone());
                set
            }
        }
    }
}

/// Cached set of allowed CAS keys for a given organization.
///
/// This type uses interior mutability to allow for thread-safe operations; even
/// when you clone an instance the internal cache is still shared with the
/// original instance.
///
/// This type implements an LRU cache; by default the set holds up to
/// [`OrgKeySet::DEFAULT_LIMIT`] keys.
#[derive(Clone, Debug)]
#[debug("OrgKeySet(count = {})", self.0.len())]
pub struct OrgKeySet(Arc<SyncCache<Key, ()>>);

impl OrgKeySet {
    /// The default number of keys to cache per organization.
    ///
    /// If more than this number of keys are inserted into the set, the least
    /// recently used key is evicted.
    pub const DEFAULT_LIMIT: u64 = 100_000;

    /// Create a new instance with the default limit.
    pub fn new() -> Self {
        SyncCache::new(Self::DEFAULT_LIMIT as usize)
            .pipe(Arc::new)
            .pipe(Self)
    }

    /// Check if the set contains the given key.
    pub fn contains(&self, key: &Key) -> bool {
        self.0.contains_key(key)
    }

    /// Insert a key into the set.
    pub fn insert(&self, key: Key) {
        self.0.insert(key, ());
    }

    /// Insert all keys into the set.
    pub fn insert_all(&self, allowed: impl IntoIterator<Item = Key>) {
        for key in allowed {
            self.0.insert(key, ());
        }
    }
}

/// An ID uniquely identifying an organization.
#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Debug,
    Display,
    Default,
    Deserialize,
    Serialize,
)]
pub struct OrgId(u64);

impl OrgId {
    pub fn as_i64(&self) -> i64 {
        self.0 as i64
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }

    pub fn from_u64(id: u64) -> Self {
        Self(id)
    }
}

impl<S: Send + Sync> FromRequestParts<S> for OrgId {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        const ORG_ID_HEADER: &str = "x-org-id";
        let Some(header) = parts.headers.get(ORG_ID_HEADER) else {
            return Err((
                StatusCode::UNAUTHORIZED,
                const_str::format!("{ORG_ID_HEADER} header required"),
            ));
        };
        let Ok(header) = header.to_str() else {
            return Err((
                StatusCode::BAD_REQUEST,
                const_str::format!("{ORG_ID_HEADER} header must be a string"),
            ));
        };

        let Ok(parsed) = header.trim().parse::<u64>() else {
            return Err((
                StatusCode::BAD_REQUEST,
                const_str::format!("{ORG_ID_HEADER} header must be a valid unsigned number"),
            ));
        };

        Ok(OrgId::from_u64(parsed))
    }
}

/// An ID uniquely identifying an account.
#[derive(
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Debug,
    Display,
    Default,
    Deserialize,
    Serialize,
    From,
    Into,
)]
pub struct AccountId(u64);

impl AccountId {
    pub fn as_i64(&self) -> i64 {
        self.0 as i64
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }

    pub fn from_i64(id: i64) -> Self {
        Self(id as u64)
    }

    pub fn from_u64(id: u64) -> Self {
        Self(id)
    }
}

/// An authenticated token, which has been validated.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AuthenticatedToken {
    /// The account ID in the database.
    pub account_id: AccountId,

    /// The organization ID in the database.
    pub org_id: OrgId,

    /// The token that was authenticated.
    pub token: RawToken,
}

impl AuthenticatedToken {
    /// Convert into a stateless representation of the authenticated token.
    pub fn into_stateless(self) -> StatelessToken {
        StatelessToken {
            account_id: self.account_id,
            org_id: self.org_id,
            token: self.token,
        }
    }
}

impl From<AuthenticatedToken> for RawToken {
    fn from(val: AuthenticatedToken) -> Self {
        val.token
    }
}

impl AsRef<RawToken> for AuthenticatedToken {
    fn as_ref(&self) -> &RawToken {
        &self.token
    }
}

/// An unauthenticated token.
///
/// These are provided by the client and have not yet been validated.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Deserialize, Serialize)]
#[debug("RawToken(..)")]
pub struct RawToken(String);

impl RawToken {
    /// Create a new raw token.
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// View the token as a string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl<S: Into<String>> From<S> for RawToken {
    fn from(token: S) -> Self {
        Self::new(token)
    }
}

impl<S: Send + Sync> FromRequestParts<S> for RawToken {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let Some(header) = parts.headers.get(AUTHORIZATION) else {
            return Err((StatusCode::UNAUTHORIZED, "Authorization header required"));
        };
        let Ok(token) = header.to_str() else {
            return Err((
                StatusCode::BAD_REQUEST,
                "Authorization header must be a string",
            ));
        };

        let token = match token.strip_prefix("Bearer") {
            Some(token) => token.trim(),
            None => token.trim(),
        };
        if token.is_empty() {
            return Err((StatusCode::BAD_REQUEST, "Empty authorization token"));
        }

        Ok(RawToken::new(token))
    }
}
