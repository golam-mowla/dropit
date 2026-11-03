use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Storage and verification for single-use session tokens.
#[derive(Clone, Default)]
pub struct SessionTokenStore {
    tokens: Arc<Mutex<HashMap<String, Instant>>>,
    ttl: Duration,
}

impl SessionTokenStore {
    /// Creates a new token store with a default 5-minute TTL.
    pub fn new() -> Self {
        Self {
            tokens: Arc::new(Mutex::new(HashMap::new())),
            ttl: Duration::from_secs(300),
        }
    }

    /// Creates a token store with custom TTL.
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            tokens: Arc::new(Mutex::new(HashMap::new())),
            ttl,
        }
    }

    /// Generates and stores a new single-use token.
    pub fn generate_token(&self) -> String {
        let token = format!(
            "{:016x}{:016x}",
            rand::random::<u64>(),
            rand::random::<u64>()
        );
        let mut map = self.tokens.lock().unwrap();
        map.insert(token.clone(), Instant::now());
        token
    }

    /// Verifies and immediately consumes a token.
    /// Returns true if valid and unexpired; false otherwise.
    pub fn verify_and_consume(&self, token: &str) -> bool {
        let mut map = self.tokens.lock().unwrap();
        if let Some(created_at) = map.remove(token) {
            created_at.elapsed() <= self.ttl
        } else {
            false
        }
    }

    /// Cleans up expired tokens.
    pub fn prune_expired(&self) {
        let mut map = self.tokens.lock().unwrap();
        map.retain(|_, created_at| created_at.elapsed() <= self.ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_generate_and_consume() {
        let store = SessionTokenStore::new();
        let token = store.generate_token();
        assert!(!token.is_empty());

        // First consumption succeeds
        assert!(store.verify_and_consume(&token));

        // Second consumption fails (single-use)
        assert!(!store.verify_and_consume(&token));
    }

    #[test]
    fn test_token_expiration() {
        let store = SessionTokenStore::with_ttl(Duration::from_millis(10));
        let token = store.generate_token();
        std::thread::sleep(Duration::from_millis(25));
        assert!(!store.verify_and_consume(&token));
    }
}
