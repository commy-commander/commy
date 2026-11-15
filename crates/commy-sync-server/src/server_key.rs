use commy_core::server_key::PublishedKey;
use commy_core::Identity;

use crate::config::Config;
use crate::store::{Store, StoreError};

pub async fn load(config: &Config, store: &Store) -> Result<Identity, StoreError> {
    if let Some(seed) = &config.server_key {
        return Ok(Identity::from_bytes(&seed.0));
    }
    let seed = store.server_key_seed(Identity::generate().to_bytes()).await?;
    Ok(Identity::from_bytes(&seed))
}

pub fn published(identity: &Identity) -> PublishedKey {
    PublishedKey::of(identity.public())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::KeySeed;

    fn config(seed: Option<[u8; 32]>) -> Config {
        let mut config = Config::from_env(|_| None).unwrap();
        config.server_key = seed.map(KeySeed);
        config
    }

    #[tokio::test]
    async fn a_set_seed_wins_and_gives_a_well_formed_published_key() {
        let key = load(&config(Some([7; 32])), &Store::memory()).await.unwrap();
        assert_eq!(key.public(), Identity::from_bytes(&[7; 32]).public());
        assert!(published(&key).is_well_formed());
    }

    #[tokio::test]
    async fn without_a_seed_a_fresh_key_is_made() {
        let one = load(&config(None), &Store::memory()).await.unwrap();
        let two = load(&config(None), &Store::memory()).await.unwrap();
        assert_ne!(one.public(), two.public());
    }
}
