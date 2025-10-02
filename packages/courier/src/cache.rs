use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

const MAX_KEYS_PER_ORG: usize = 100_000;

/// In-memory cache of allowed CAS keys per organization
/// Structure: HashMap<OrgId, LruHashSet<Blake3>>
pub struct KeyCache {
    cache: Arc<RwLock<HashMap<i64, OrgKeySet>>>,
}

struct OrgKeySet {
    keys: Vec<Vec<u8>>, // TODO: Use a proper LRU structure
    session_count: usize,
}

impl KeyCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn preload_keys(&self, org_id: i64, keys: Vec<Vec<u8>>) {
        todo!("1. Acquire write lock");
        todo!("2. If org_id already has a key set, increment session_count");
        todo!("3. If not, create new OrgKeySet with keys and session_count=1");
        todo!("4. Limit keys to MAX_KEYS_PER_ORG");
    }

    pub async fn contains_key(&self, org_id: i64, key: &[u8]) -> bool {
        todo!("1. Acquire read lock");
        todo!("2. Check if key exists in org's key set");
    }

    pub async fn insert_key(&self, org_id: i64, key: Vec<u8>) {
        todo!("1. Acquire write lock");
        todo!("2. Insert key into org's key set");
        todo!("3. Evict oldest key if over MAX_KEYS_PER_ORG (LRU)");
    }

    pub async fn decrement_session(&self, org_id: i64) {
        todo!("1. Acquire write lock");
        todo!("2. Decrement session_count for org");
        todo!("3. If session_count reaches 0, remove entire org key set");
    }
}
