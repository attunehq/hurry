use aerosol::axum::Dep;
use axum::{Json, http::StatusCode, response::IntoResponse};
use color_eyre::eyre::Report;
use serde::Serialize;
use tracing::{info, warn};

use crate::{
    auth::{KeySets, OrgId, OrgKeySet, RawToken, StatelessToken},
    db::Postgres,
};

/// Uses the user API token to validate their authentication and org membership,
/// then loads the user's most frequently accessed CAS keys into the in-memory
/// allowed key set and mints a stateless token that allows access to those
/// keys.
///
/// The intention here is that:
/// - Clients (hurry) will need to access a large number of keys
/// - We shouldn't make clients pay for the latency of checking the database on
///   each request, as one of the top priorities for this service is latency.
///
/// ## Implementation
///
/// We use PARETO tokens to authenticate the request. PARETO tokens are a sort
/// of upgraded JWT; since we implement both ends we don't need to worry too
/// much about interoperability with other libraries.
///
/// The secret used to sign the token is generated from random data at API
/// server startup; since it guards access to a memory-resident cache it doesn't
/// really matter if it's persistent since the cache is wiped out if the server
/// restarts anyway.
///
/// ## Preloading
///
/// The user's most frequently accessed CAS keys are loaded into the in-memory
/// allowed key set when the token is minted. While normally API servers strive
/// to be stateless, in this implementation we're baking in the assumption that
/// clients are routed to a stable set of backends based on their org ID headers
/// by the ingress, so we can safely store _some_ state.
///
/// ## Expiration
///
/// The stateless token will expire after 1 hour, which is the default
/// expiration time for the PARETO token implementation we use. We may change
/// this in the future, but for now since we have a LRU cache of organizations
/// it doesn't seem to matter _that_ much (old idle orgs will get evicted as
/// needed).
///
/// ## Backup authentication
///
/// If the key isn't in the set of preloaded keys, the server checks the
/// database and stores the key in the set. Each set uses an LRU cache of
/// allowed keys, so this memory usage is bounded.
pub async fn handle(
    token: RawToken,
    org_id: OrgId,
    Dep(keysets): Dep<KeySets>,
    Dep(db): Dep<Postgres>,
) -> MintStatelessResponse {
    match db.validate(org_id, token).await {
        Ok(None) => MintStatelessResponse::Unauthorized,
        Ok(Some(token)) => {
            let allowed = db
                .user_allowed_cas_keys(token.user_id, OrgKeySet::DEFAULT_LIMIT)
                .await;
            match allowed {
                Ok(allowed) => {
                    info!(allowed = ?allowed.len(), user = ?token.user_id, org = ?token.org_id, "insert allowed cas keys");
                    keysets.organization(org_id).insert_all(allowed);
                }
                Err(error) => {
                    warn!(?error, user = ?token.user_id, "get allowed cas keys");
                }
            }
            MintStatelessResponse::Success(token.into_stateless())
        }
        Err(error) => MintStatelessResponse::Error(error),
    }
}

#[derive(Debug, Serialize)]
pub struct MintStatelessResponseBody {
    pub token: StatelessToken,
}

#[derive(Debug)]
pub enum MintStatelessResponse {
    Unauthorized,
    Success(StatelessToken),
    Error(Report),
}

impl IntoResponse for MintStatelessResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            MintStatelessResponse::Unauthorized => StatusCode::UNAUTHORIZED.into_response(),
            MintStatelessResponse::Success(token) => {
                (StatusCode::OK, Json(MintStatelessResponseBody { token })).into_response()
            }
            MintStatelessResponse::Error(error) => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:?}")).into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use color_eyre::{Result, eyre::Context};
    use pretty_assertions::assert_eq as pretty_assert_eq;
    use serde_json::Value;
    use sqlx::PgPool;

    use crate::auth::{OrgId, RawToken, StatelessToken, UserId};

    #[sqlx::test(
        migrator = "crate::db::Postgres::MIGRATOR",
        fixtures("../../../../schema/fixtures/auth.sql")
    )]
    async fn test_mint_stateless_token(pool: PgPool) -> Result<()> {
        const TOKEN: &str = "test-token:user1@test1.com";
        let (server, _tmp) = crate::api::test_server(pool)
            .await
            .context("create test server")?;

        let response = server
            .post("/api/v1/auth")
            .add_header("Authorization", format!("Bearer {TOKEN}"))
            .add_header("x-org-id", "1")
            .await;

        response.assert_status_ok();
        let body = response.json::<Value>();
        let token = body["token"].as_str().expect("token as a string");

        let stateless = StatelessToken::deserialize(token).expect("deserialize token");
        pretty_assert_eq!(stateless.org_id, OrgId::from_u64(1));
        pretty_assert_eq!(stateless.user_id, UserId::from_u64(1));
        pretty_assert_eq!(stateless.token, RawToken::new(TOKEN));

        Ok(())
    }
}
