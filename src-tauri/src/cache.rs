use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::types::{Provider, UsageResponse};

const TTL: Duration = Duration::from_secs(30);

struct Entry {
    value: UsageResponse,
    at: Instant,
}

pub struct UsageCache {
    map: Mutex<HashMap<Provider, Entry>>,
}

impl UsageCache {
    pub fn new() -> Self {
        Self { map: Mutex::new(HashMap::new()) }
    }

    pub fn get(&self, provider: Provider) -> Option<UsageResponse> {
        let map = self.map.lock().unwrap();
        let entry = map.get(&provider)?;
        if entry.at.elapsed() < TTL {
            Some(entry.value.clone())
        } else {
            None
        }
    }

    pub fn put(&self, provider: Provider, value: UsageResponse) {
        let mut map = self.map.lock().unwrap();
        map.insert(provider, Entry { value, at: Instant::now() });
    }

    pub fn invalidate(&self, provider: Provider) {
        self.map.lock().unwrap().remove(&provider);
    }

    pub fn clear(&self) {
        self.map.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Status, UsageResponse};

    fn make_resp(provider: Provider) -> UsageResponse {
        UsageResponse {
            provider,
            status: Status::Ok,
            windows: vec![],
            extra_usage: None,
            error: None,
        }
    }

    #[test]
    fn put_and_get_within_ttl() {
        let cache = UsageCache::new();
        cache.put(Provider::Claude, make_resp(Provider::Claude));
        assert!(cache.get(Provider::Claude).is_some());
    }

    #[test]
    fn separate_providers_do_not_collide() {
        let cache = UsageCache::new();
        cache.put(Provider::Claude, make_resp(Provider::Claude));
        assert!(cache.get(Provider::Codex).is_none());
    }

    #[test]
    fn invalidate_removes_entry() {
        let cache = UsageCache::new();
        cache.put(Provider::Claude, make_resp(Provider::Claude));
        cache.invalidate(Provider::Claude);
        assert!(cache.get(Provider::Claude).is_none());
    }
}
