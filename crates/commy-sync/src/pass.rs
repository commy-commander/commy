use rand::{CryptoRng, RngCore};
use rsa::pkcs8::DecodePublicKey;
use rsa::traits::PublicKeyParts;
use rsa::{BigUint, Pss, RsaPublicKey};
use serde::{Deserialize, Serialize};
use sha2_10::{Digest, Sha384};

use crate::wire::{Blob, Bytes};

pub const PASS_KEY_BITS: usize = 2048;
pub const PASS_TOKEN_BYTES: usize = 32;
pub const PASS_RANDOMIZER_BYTES: usize = 32;
pub const EPOCH_DAYS: i64 = 7;
pub const PASS_KEYS_PATH: &str = "/api/auth/commy/passes/keys";
pub const PASS_ISSUE_PATH: &str = "/api/auth/commy/passes";
pub const MAX_PASSES_PER_REQUEST: usize = 10;
const HASH_BYTES: usize = 48;
const SALT_BYTES: usize = 48;
const PUBLIC_EXPONENT: u32 = 65_537;
const PASS_LABEL: &[u8] = b"commy-sync-new-stream-pass/v1";
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pass {
    pub epoch: u32,
    pub token: Bytes<PASS_TOKEN_BYTES>,
    pub randomizer: Bytes<PASS_RANDOMIZER_BYTES>,
    pub sig: Blob,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PassError {
    #[error("the pass key is not a {PASS_KEY_BITS}-bit RSA key with exponent {PUBLIC_EXPONENT}")]
    BadKey,
    #[error("the blind signature does not fit the pass key")]
    BadSignature,
    #[error("the pass key cannot sign this message")]
    Unsignable,
}

pub fn epoch_of(now_ms: i64) -> u32 {
    (now_ms.max(0) / DAY_MS / EPOCH_DAYS) as u32
}

pub struct PendingPass {
    epoch: u32,
    token: [u8; PASS_TOKEN_BYTES],
    randomizer: [u8; PASS_RANDOMIZER_BYTES],
    unblind: BigUint,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PassKey {
    epoch: u32,
    key: RsaPublicKey,
}

impl PassKey {
    pub fn from_der(epoch: u32, der: &[u8]) -> Result<Self, PassError> {
        let key = RsaPublicKey::from_public_key_der(der).map_err(|_| PassError::BadKey)?;
        if key.n().bits() != PASS_KEY_BITS || *key.e() != BigUint::from(PUBLIC_EXPONENT) {
            return Err(PassError::BadKey);
        }
        Ok(Self { epoch, key })
    }

    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn verify(&self, pass: &Pass) -> bool {
        if pass.epoch != self.epoch {
            return false;
        }
        let hashed = Sha384::digest(prepared(&pass.randomizer.0, pass.epoch, &pass.token.0));
        self.key.verify(Pss::new_with_salt::<Sha384>(SALT_BYTES), &hashed, &pass.sig.0).is_ok()
    }

    pub fn blind(&self, rng: &mut (impl RngCore + CryptoRng)) -> Result<(Vec<u8>, PendingPass), PassError> {
        let mut token = [0u8; PASS_TOKEN_BYTES];
        let mut randomizer = [0u8; PASS_RANDOMIZER_BYTES];
        rng.fill_bytes(&mut token);
        rng.fill_bytes(&mut randomizer);
        let encoded = emsa_pss_encode(&prepared(&randomizer, self.epoch, &token), self.key.n().bits() - 1, rng)?;
        let n = self.key.n();
        let m = BigUint::from_bytes_be(&encoded);
        let (factor, unblind) = blinding_factor(n, rng);
        let blinded = (m * factor.modpow(self.key.e(), n)) % n;
        Ok((padded(&blinded, self.key.size()), PendingPass { epoch: self.epoch, token, randomizer, unblind }))
    }

    pub fn finish(&self, pending: PendingPass, blind_sig: &[u8]) -> Result<Pass, PassError> {
        let n = self.key.n();
        let z = BigUint::from_bytes_be(blind_sig);
        if blind_sig.len() != self.key.size() || &z >= n || pending.epoch != self.epoch {
            return Err(PassError::BadSignature);
        }
        let sig = (z * &pending.unblind) % n;
        let pass = Pass { epoch: pending.epoch, token: Bytes(pending.token), randomizer: Bytes(pending.randomizer), sig: Blob(padded(&sig, self.key.size())) };
        if self.verify(&pass) {
            Ok(pass)
        } else {
            Err(PassError::BadSignature)
        }
    }
}

fn prepared(randomizer: &[u8; PASS_RANDOMIZER_BYTES], epoch: u32, token: &[u8; PASS_TOKEN_BYTES]) -> Vec<u8> {
    [randomizer.as_slice(), PASS_LABEL, &epoch.to_be_bytes(), token].concat()
}

fn padded(value: &BigUint, size: usize) -> Vec<u8> {
    let bytes = value.to_bytes_be();
    let mut out = vec![0u8; size.saturating_sub(bytes.len())];
    out.extend_from_slice(&bytes);
    out
}

fn blinding_factor(n: &BigUint, rng: &mut (impl RngCore + CryptoRng)) -> (BigUint, BigUint) {
    use rsa::BigUint as Big;
    let mut bytes = vec![0u8; n.to_bytes_be().len()];
    loop {
        rng.fill_bytes(&mut bytes);
        let candidate = Big::from_bytes_be(&bytes) % n;
        if candidate <= Big::from(1u8) {
            continue;
        }
        if let Some(inverse) = inverse_mod(&candidate, n) {
            return (candidate, inverse);
        }
    }
}

fn inverse_mod(value: &BigUint, n: &BigUint) -> Option<BigUint> {
    use num_bigint_dig::ModInverse;
    value.clone().mod_inverse(n.clone())?.to_biguint()
}

fn mgf1(seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + HASH_BYTES);
    let mut counter = 0u32;
    while out.len() < len {
        out.extend_from_slice(&Sha384::new().chain_update(seed).chain_update(counter.to_be_bytes()).finalize());
        counter += 1;
    }
    out.truncate(len);
    out
}

fn emsa_pss_encode(message: &[u8], em_bits: usize, rng: &mut (impl RngCore + CryptoRng)) -> Result<Vec<u8>, PassError> {
    let em_len = em_bits.div_ceil(8);
    if em_len < HASH_BYTES + SALT_BYTES + 2 {
        return Err(PassError::Unsignable);
    }
    let m_hash = Sha384::digest(message);
    let mut salt = [0u8; SALT_BYTES];
    rng.fill_bytes(&mut salt);
    let h = Sha384::new().chain_update([0u8; 8]).chain_update(m_hash).chain_update(salt).finalize();
    let db_len = em_len - HASH_BYTES - 1;
    let mut db = vec![0u8; db_len - SALT_BYTES - 1];
    db.push(0x01);
    db.extend_from_slice(&salt);
    for (byte, mask) in db.iter_mut().zip(mgf1(&h, db_len)) {
        *byte ^= mask;
    }
    db[0] &= 0xff >> (8 * em_len - em_bits);
    db.extend_from_slice(&h);
    db.push(0xbc);
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::traits::PrivateKeyParts;
    use rsa::RsaPrivateKey;
    use std::sync::OnceLock;

    fn issuer() -> &'static RsaPrivateKey {
        static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
        KEY.get_or_init(|| RsaPrivateKey::new(&mut rand::thread_rng(), PASS_KEY_BITS).unwrap())
    }

    fn key(epoch: u32) -> PassKey {
        use rsa::pkcs8::EncodePublicKey;
        let der = issuer().to_public_key().to_public_key_der().unwrap();
        PassKey::from_der(epoch, der.as_bytes()).unwrap()
    }

    fn raw_sign(blinded: &[u8]) -> Vec<u8> {
        let private = issuer();
        padded(&BigUint::from_bytes_be(blinded).modpow(private.d(), private.n()), private.size())
    }

    #[test]
    fn a_blindly_signed_pass_verifies_and_reveals_nothing_the_issuer_saw() {
        let key = key(2_900);
        let (blinded, pending) = key.blind(&mut rand::thread_rng()).unwrap();
        assert_eq!(blinded.len(), PASS_KEY_BITS / 8);
        let pass = key.finish(pending, &raw_sign(&blinded)).unwrap();
        assert!(key.verify(&pass));
        assert!(!blinded.windows(PASS_TOKEN_BYTES).any(|w| w == pass.token.0), "the issuer never sees the token");
        assert_ne!(blinded, pass.sig.0, "the signature it made is not the one that is spent");
    }

    #[test]
    fn a_changed_or_misdated_pass_fails() {
        let key = key(7);
        let (blinded, pending) = key.blind(&mut rand::thread_rng()).unwrap();
        let pass = key.finish(pending, &raw_sign(&blinded)).unwrap();
        let mut other_token = pass.clone();
        other_token.token.0[0] ^= 1;
        assert!(!key.verify(&other_token));
        let mut other_epoch = pass.clone();
        other_epoch.epoch = 8;
        assert!(!key.verify(&other_epoch));
        assert!(!self::key(8).verify(&pass), "a pass only counts in the week it was made for");
        let mut bent = pass;
        bent.sig.0[10] ^= 1;
        assert!(!key.verify(&bent));
    }

    #[test]
    fn a_wrong_blind_signature_is_refused_before_it_is_kept() {
        let key = key(1);
        let (blinded, pending) = key.blind(&mut rand::thread_rng()).unwrap();
        let mut wrong = raw_sign(&blinded);
        wrong[5] ^= 1;
        assert_eq!(key.finish(pending, &wrong).err(), Some(PassError::BadSignature));
        let (_, pending) = key.blind(&mut rand::thread_rng()).unwrap();
        assert_eq!(key.finish(pending, &[0u8; 10]).err(), Some(PassError::BadSignature));
    }

    #[test]
    fn only_full_size_keys_are_taken() {
        use rsa::pkcs8::EncodePublicKey;
        let small = RsaPrivateKey::new(&mut rand::thread_rng(), 1024).unwrap();
        let der = small.to_public_key().to_public_key_der().unwrap();
        assert_eq!(PassKey::from_der(1, der.as_bytes()).err(), Some(PassError::BadKey));
        assert_eq!(PassKey::from_der(1, b"junk").err(), Some(PassError::BadKey));
    }

    #[test]
    fn weeks_count_from_the_epoch() {
        assert_eq!(epoch_of(0), 0);
        assert_eq!(epoch_of(7 * DAY_MS - 1), 0);
        assert_eq!(epoch_of(7 * DAY_MS), 1);
        assert_eq!(epoch_of(-5), 0);
    }

    #[test]
    fn passes_travel_as_tagged_json() {
        let pass = Pass { epoch: 3, token: Bytes([1; 32]), randomizer: Bytes([2; 32]), sig: Blob(vec![3; 256]) };
        let json = serde_json::to_string(&pass).unwrap();
        assert_eq!(serde_json::from_str::<Pass>(&json).unwrap(), pass);
    }
}
