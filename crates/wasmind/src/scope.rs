#[cfg(feature = "test-utils")]
use std::sync::atomic::AtomicU64;
#[cfg(feature = "test-utils")]
use std::sync::atomic::Ordering;

/// A scope identifier - a 6-character alphanumeric string used to identify agent contexts
pub type Scope = String;

#[cfg(feature = "test-utils")]
static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Generate a new 6-character alphanumeric scope identifier
pub fn new_scope() -> Scope {
    #[cfg(feature = "test-utils")]
    {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("{:06X}", counter % 0xFFFFFF)
    }
    #[cfg(not(feature = "test-utils"))]
    {
        use rand::Rng;
        const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        let mut rng = rand::rng();
        (0..6)
            .map(|_| {
                let idx = rng.random_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_scope_generates_6_char_scope() {
        let scope = new_scope();
        assert_eq!(scope.len(), 6);
        assert!(scope.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    #[cfg(feature = "test-utils")]
    fn test_deterministic_scope_in_tests() {
        let scope1 = new_scope();
        let scope2 = new_scope();
        assert_ne!(scope1, scope2);
        assert_eq!(scope1.len(), 6);
        assert_eq!(scope2.len(), 6);
    }
}
