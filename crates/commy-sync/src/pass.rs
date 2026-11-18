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
pub const WALLET_TARGET: usize = 10;
pub const WALLET_LOW: usize = 4;
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
    #[error("the account server lists no pass key for this week")]
    NoKey,
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

#[derive(Deserialize)]
struct PublishedKeys {
    keys: Vec<PublishedKey>,
}

#[derive(Deserialize)]
struct PublishedKey {
    epoch: u32,
    der: String,
}

pub fn current_key(published: &str, now_ms: i64) -> Result<PassKey, PassError> {
    let epoch = epoch_of(now_ms);
    let keys: PublishedKeys = serde_json::from_str(published).map_err(|_| PassError::NoKey)?;
    let key = keys.keys.into_iter().find(|k| k.epoch == epoch).ok_or(PassError::NoKey)?;
    let der = data_encoding::BASE64URL_NOPAD.decode(key.der.as_bytes()).map_err(|_| PassError::BadKey)?;
    PassKey::from_der(epoch, &der)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueRequest {
    pub epoch: u32,
    pub blinded: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IssueReply {
    pub epoch: u32,
    pub signatures: Vec<String>,
}

pub fn ask(key: &PassKey, count: usize, rng: &mut (impl RngCore + CryptoRng)) -> Result<(IssueRequest, Vec<PendingPass>), PassError> {
    let mut blinded = Vec::new();
    let mut pending = Vec::new();
    for _ in 0..count.min(MAX_PASSES_PER_REQUEST) {
        let (message, waiting) = key.blind(rng)?;
        blinded.push(data_encoding::BASE64URL_NOPAD.encode(&message));
        pending.push(waiting);
    }
    Ok((IssueRequest { epoch: key.epoch, blinded }, pending))
}

pub fn collect(key: &PassKey, pending: Vec<PendingPass>, reply: &IssueReply) -> Vec<Pass> {
    if reply.epoch != key.epoch {
        return Vec::new();
    }
    pending
        .into_iter()
        .zip(&reply.signatures)
        .filter_map(|(waiting, sig)| key.finish(waiting, &data_encoding::BASE64URL_NOPAD.decode(sig.as_bytes()).ok()?).ok())
        .collect()
}

pub fn same_issuer(a: &str, b: &str) -> bool {
    a.trim_end_matches('/').eq_ignore_ascii_case(b.trim_end_matches('/'))
}

fn usable(pass: &Pass, now_ms: i64) -> bool {
    let now = epoch_of(now_ms);
    pass.epoch == now || pass.epoch.checked_add(1) == Some(now)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wallet {
    pub issuer: String,
    pub passes: Vec<Pass>,
}

impl Wallet {
    pub fn new(issuer: &str) -> Self {
        Self { issuer: issuer.trim_end_matches('/').to_string(), passes: Vec::new() }
    }

    pub fn resume(saved: Option<Wallet>, issuer: &str) -> Self {
        saved.filter(|w| same_issuer(&w.issuer, issuer)).unwrap_or_else(|| Wallet::new(issuer))
    }

    pub fn take(&mut self, issuer: &str, now_ms: i64) -> Option<Pass> {
        if !same_issuer(&self.issuer, issuer) {
            return None;
        }
        self.passes.retain(|pass| usable(pass, now_ms));
        self.passes.sort_by_key(|pass| pass.epoch);
        (!self.passes.is_empty()).then(|| self.passes.remove(0))
    }

    pub fn wanted(&self, now_ms: i64) -> usize {
        let fresh = self.passes.iter().filter(|pass| pass.epoch == epoch_of(now_ms)).count();
        if fresh >= WALLET_LOW {
            0
        } else {
            WALLET_TARGET - fresh
        }
    }

    pub fn add(&mut self, passes: Vec<Pass>, now_ms: i64) {
        self.passes.extend(passes);
        self.passes.retain(|pass| usable(pass, now_ms));
        self.passes.sort_by_key(|pass| pass.epoch);
        let extra = self.passes.len().saturating_sub(WALLET_TARGET * 2);
        self.passes.drain(..extra);
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

    fn published(epochs: &[u32]) -> String {
        use rsa::pkcs8::EncodePublicKey;
        let der = issuer().to_public_key().to_public_key_der().unwrap();
        let keys: Vec<serde_json::Value> =
            epochs.iter().map(|e| serde_json::json!({ "epoch": e, "der": data_encoding::BASE64URL_NOPAD.encode(der.as_bytes()) })).collect();
        serde_json::json!({ "keys": keys }).to_string()
    }

    const WEEK_MS: i64 = EPOCH_DAYS * DAY_MS;

    #[test]
    fn a_batch_is_asked_for_and_collected_against_this_weeks_key() {
        let now = 40 * WEEK_MS + 5;
        assert_eq!(current_key(&published(&[39, 41]), now).err(), Some(PassError::NoKey));
        assert_eq!(current_key("junk", now).err(), Some(PassError::NoKey));
        let key = current_key(&published(&[39, 40, 41]), now).unwrap();
        assert_eq!(key.epoch(), 40);
        let (request, pending) = ask(&key, MAX_PASSES_PER_REQUEST + 5, &mut rand::thread_rng()).unwrap();
        assert_eq!((request.epoch, request.blinded.len(), pending.len()), (40, MAX_PASSES_PER_REQUEST, MAX_PASSES_PER_REQUEST));
        let mut signatures: Vec<String> = request
            .blinded
            .iter()
            .map(|b| data_encoding::BASE64URL_NOPAD.encode(&raw_sign(&data_encoding::BASE64URL_NOPAD.decode(b.as_bytes()).unwrap())))
            .collect();
        signatures.truncate(7);
        signatures[0] = "AAAA".into();
        let passes = collect(&key, pending, &IssueReply { epoch: 40, signatures });
        assert_eq!(passes.len(), 6, "a short or broken answer keeps only what checks out");
        assert!(passes.iter().all(|p| key.verify(p)));
        let (_, pending) = ask(&key, 1, &mut rand::thread_rng()).unwrap();
        assert!(collect(&key, pending, &IssueReply { epoch: 41, signatures: vec![] }).is_empty());
    }

    #[test]
    fn a_wallet_spends_oldest_first_and_only_for_its_issuer() {
        let now = 40 * WEEK_MS;
        let pass = |epoch: u32| Pass { epoch, token: Bytes(rand::random()), randomizer: Bytes([2; 32]), sig: Blob(vec![3; 256]) };
        let mut wallet = Wallet::new("https://accounts.example.org/");
        assert_eq!(wallet.wanted(now), WALLET_TARGET);
        wallet.add(vec![pass(40), pass(39), pass(38), pass(40)], now);
        assert_eq!(wallet.passes.len(), 3, "a pass two weeks old is dropped");
        assert_eq!(wallet.take("https://other.example.org", now), None);
        assert_eq!(wallet.take("https://ACCOUNTS.example.org", now).unwrap().epoch, 39, "last week's go first");
        assert_eq!(wallet.wanted(now), WALLET_TARGET - 2);
        wallet.add((0..WALLET_TARGET * 3).map(|_| pass(40)).collect(), now);
        assert_eq!(wallet.passes.len(), WALLET_TARGET * 2);
        assert_eq!(wallet.wanted(now), 0);
        assert_eq!(wallet.wanted(now + WEEK_MS), WALLET_TARGET, "a new week asks for fresh ones while last week's still count");
        assert!(wallet.take("https://accounts.example.org", now + WEEK_MS).is_some());
        assert_eq!(wallet.take("https://accounts.example.org", now + 2 * WEEK_MS), None);
        let json = serde_json::to_string(&wallet).unwrap();
        assert_eq!(serde_json::from_str::<Wallet>(&json).unwrap(), wallet);
        assert_eq!(Wallet::resume(Some(wallet.clone()), "https://accounts.example.org"), wallet);
        assert_eq!(Wallet::resume(Some(wallet), "https://other.example.org"), Wallet::new("https://other.example.org"));
        assert_eq!(Wallet::resume(None, "https://other.example.org/"), Wallet::new("https://other.example.org"));
    }

    #[test]
    fn passes_travel_as_tagged_json() {
        let pass = Pass { epoch: 3, token: Bytes([1; 32]), randomizer: Bytes([2; 32]), sig: Blob(vec![3; 256]) };
        let json = serde_json::to_string(&pass).unwrap();
        assert_eq!(serde_json::from_str::<Pass>(&json).unwrap(), pass);
    }
}
