use crate::sealed::{Purpose, SealKey};

pub const VAULT_ITEM: &str = "entry-key";
pub const LOCK_ID: &str = "656e7472792d6b65792d657363726f77";

pub fn wrapping_key(server_key_hex: &str) -> Option<SealKey> {
    let bytes = data_encoding::HEXLOWER_PERMISSIVE.decode(server_key_hex.trim().as_bytes()).ok()?;
    Some(SealKey::from_bytes(bytes.try_into().ok()?))
}

pub fn wrap(wrapping: &SealKey, key: &SealKey) -> String {
    wrapping.seal(Purpose::EntryKeyEscrow, key.to_text().as_bytes())
}

pub fn unwrap(wrapping: &SealKey, sealed: &str) -> Option<SealKey> {
    let text = wrapping.open(Purpose::EntryKeyEscrow, sealed).ok()?;
    SealKey::from_text(std::str::from_utf8(&text).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_copy_opens_with_the_same_server_key_only() {
        let server_key = "ab".repeat(32);
        let wrapping = wrapping_key(&server_key).unwrap();
        let key = SealKey::generate();
        let stored = wrap(&wrapping, &key);
        assert!(stored.starts_with(crate::sealed::SEALED_PREFIX));
        assert!(!stored.contains(&key.to_text()));
        assert_eq!(unwrap(&wrapping_key(&server_key.to_uppercase()).unwrap(), &stored), Some(key));
        assert_eq!(unwrap(&wrapping_key(&"cd".repeat(32)).unwrap(), &stored), None);
        assert!(wrapping_key("abc").is_none());
    }

    #[test]
    fn the_lock_id_is_one_the_account_server_takes() {
        assert_eq!(LOCK_ID.len(), 32);
        assert!(LOCK_ID.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }
}
