use color_eyre::Result;
use jsonwebtoken::{DecodingKey, EncodingKey};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub user_id: i64,
    pub org_id: i64,
    pub exp: i64,
}

pub struct JwtManager {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

impl JwtManager {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret),
            decoding_key: DecodingKey::from_secret(secret),
        }
    }

    pub fn mint(&self, _user_id: i64, _org_id: i64, _expires_in_seconds: i64) -> Result<String> {
        todo!("1. Create Claims with user_id, org_id, exp");
        todo!("2. Encode JWT with jsonwebtoken::encode");
    }

    pub fn validate(&self, _token: &str) -> Result<Claims> {
        todo!("1. Decode JWT with jsonwebtoken::decode");
        todo!("2. Validate expiration, signature");
        todo!("3. Return Claims");
    }
}
