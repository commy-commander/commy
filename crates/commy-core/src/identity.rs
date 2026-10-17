use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::io::{self, Write};
use std::path::Path;

pub const IDENTITY_FILE: &str = "identity.key";

#[derive(Clone)]
pub struct Identity(SigningKey);

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdentityKey(pub [u8; 32]);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Signature(pub [u8; 64]);

impl Identity {
    pub fn generate() -> Self {
        Self::from_bytes(&rand::random::<[u8; 32]>())
    }

    pub fn from_bytes(secret: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(secret))
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    pub fn public(&self) -> IdentityKey {
        IdentityKey(self.0.verifying_key().to_bytes())
    }

    pub fn sign(&self, msg: &[u8]) -> Signature {
        Signature(self.0.sign(msg).to_bytes())
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Identity").field(&self.public()).finish()
    }
}

impl IdentityKey {
    pub fn verify(&self, msg: &[u8], sig: &Signature) -> bool {
        let Ok(key) = VerifyingKey::from_bytes(&self.0) else { return false };
        let sig = ed25519_dalek::Signature::from_bytes(&sig.0);
        key.verify_strict(msg, &sig).is_ok()
    }

    pub fn to_hex(&self) -> String {
        to_hex(&self.0)
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        from_hex(s).map(Self)
    }
}

impl Signature {
    pub fn to_hex(&self) -> String {
        to_hex(&self.0)
    }

    pub fn from_hex(s: &str) -> Option<Self> {
        from_hex(s).map(Self)
    }
}

impl fmt::Debug for IdentityKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IdentityKey({})", self.to_hex())
    }
}

impl fmt::Display for IdentityKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({})", self.to_hex())
    }
}

pub fn load_or_create(dir: &Path) -> io::Result<Identity> {
    let path = dir.join(IDENTITY_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let parsed = std::str::from_utf8(&bytes).ok().and_then(|s| from_hex::<32>(s.trim()));
            if let Some(secret) = parsed {
                return Ok(Identity::from_bytes(&secret));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let identity = Identity::generate();
    write_secret_atomically(dir, &path, &to_hex(&identity.to_bytes()))?;
    Ok(identity)
}

fn write_secret_atomically(dir: &Path, path: &Path, contents: &str) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{IDENTITY_FILE}.{}.tmp", to_hex(&rand::random::<[u8; 6]>())));
    let result = (|| {
        write_private(&tmp, contents.as_bytes())?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

pub(crate) fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    out
}

pub(crate) fn from_hex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let s = s.as_bytes();
    if s.len() != N * 2 {
        return None;
    }
    let nibble = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = [0u8; N];
    for (i, pair) in s.chunks_exact(2).enumerate() {
        out[i] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(out)
}

fn serialize_fixed<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    if serializer.is_human_readable() {
        serializer.serialize_str(&to_hex(bytes))
    } else {
        serializer.serialize_bytes(bytes)
    }
}

fn deserialize_fixed<'de, D: Deserializer<'de>, const N: usize>(deserializer: D) -> Result<[u8; N], D::Error> {
    struct FixedVisitor<const N: usize>;

    impl<'de, const N: usize> Visitor<'de> for FixedVisitor<N> {
        type Value = [u8; N];

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{N} bytes or {} hex characters", N * 2)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            from_hex(v).ok_or_else(|| E::invalid_value(de::Unexpected::Str(v), &self))
        }

        fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
            v.try_into().map_err(|_| E::invalid_length(v.len(), &self))
        }

        fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut out = [0u8; N];
            for (i, slot) in out.iter_mut().enumerate() {
                *slot = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(i, &self))?;
            }
            if seq.next_element::<u8>()?.is_some() {
                return Err(de::Error::invalid_length(N + 1, &self));
            }
            Ok(out)
        }
    }

    if deserializer.is_human_readable() {
        deserializer.deserialize_str(FixedVisitor::<N>)
    } else {
        deserializer.deserialize_bytes(FixedVisitor::<N>)
    }
}

impl Serialize for IdentityKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_fixed(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for IdentityKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_fixed(deserializer).map(Self)
    }
}

impl Serialize for Signature {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_fixed(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserialize_fixed(deserializer).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("commy-identity-tests")
            .join(format!("{tag}-{}", to_hex(&rand::random::<[u8; 8]>())));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sign_and_verify() {
        let id = Identity::generate();
        let sig = id.sign(b"hello");
        assert!(id.public().verify(b"hello", &sig));
        assert!(!id.public().verify(b"hellO", &sig));
        assert!(!Identity::generate().public().verify(b"hello", &sig), "other key");
        let mut bad = sig;
        bad.0[0] ^= 1;
        assert!(!id.public().verify(b"hello", &bad));
    }

    #[test]
    fn invalid_public_key_never_verifies() {
        let id = Identity::generate();
        let sig = id.sign(b"x");
        let bogus = IdentityKey([0xff; 32]);
        assert!(!bogus.verify(b"x", &sig));
        assert!(!IdentityKey([0; 32]).verify(b"x", &sig));
    }

    #[test]
    fn secret_bytes_roundtrip() {
        let id = Identity::generate();
        let back = Identity::from_bytes(&id.to_bytes());
        assert_eq!(back.public(), id.public());
        assert_ne!(Identity::generate().public(), id.public());
    }

    #[test]
    fn debug_hides_the_secret() {
        let id = Identity::generate();
        let shown = format!("{id:?}");
        assert!(shown.contains(&id.public().to_hex()));
        assert!(!shown.contains(&to_hex(&id.to_bytes())));
    }

    #[test]
    fn hex_roundtrip_and_validation() {
        let key = Identity::generate().public();
        let hex = key.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(hex, hex.to_lowercase());
        assert_eq!(IdentityKey::from_hex(&hex), Some(key));
        assert_eq!(IdentityKey::from_hex(&hex.to_uppercase()), Some(key));
        assert_eq!(IdentityKey::from_hex(&hex[..62]), None);
        assert_eq!(IdentityKey::from_hex(&format!("{}zz", &hex[..62])), None);
        assert_eq!(IdentityKey::from_hex(""), None);
        let sig = Identity::generate().sign(b"m");
        assert_eq!(Signature::from_hex(&sig.to_hex()), Some(sig));
        assert_eq!(key.to_string(), hex);
    }

    #[test]
    fn json_uses_lowercase_hex() {
        let id = Identity::generate();
        let key = id.public();
        let json = serde_json::to_string(&key).unwrap();
        assert_eq!(json, format!("\"{}\"", key.to_hex()));
        assert_eq!(serde_json::from_str::<IdentityKey>(&json).unwrap(), key);
        let sig = id.sign(b"m");
        let json = serde_json::to_string(&sig).unwrap();
        assert_eq!(json.len(), 128 + 2);
        assert_eq!(serde_json::from_str::<Signature>(&json).unwrap(), sig);
        assert!(serde_json::from_str::<IdentityKey>("\"abcd\"").is_err());
        assert!(serde_json::from_str::<Signature>(&format!("\"{}\"", key.to_hex())).is_err());
    }

    #[test]
    fn postcard_uses_raw_bytes() {
        let id = Identity::generate();
        let key = id.public();
        let bytes = postcard::to_stdvec(&key).unwrap();
        assert_eq!(bytes.len(), 33);
        assert_eq!(&bytes[1..], &key.0);
        assert_eq!(postcard::from_bytes::<IdentityKey>(&bytes).unwrap(), key);
        let sig = id.sign(b"m");
        let bytes = postcard::to_stdvec(&sig).unwrap();
        assert_eq!(bytes.len(), 65);
        assert_eq!(postcard::from_bytes::<Signature>(&bytes).unwrap(), sig);
        let short = postcard::to_stdvec(&serde_bytes_like(&key.0[..31])).unwrap();
        assert!(postcard::from_bytes::<IdentityKey>(&short).is_err());
    }

    fn serde_bytes_like(bytes: &[u8]) -> impl Serialize + '_ {
        struct Raw<'a>(&'a [u8]);
        impl Serialize for Raw<'_> {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_bytes(self.0)
            }
        }
        Raw(bytes)
    }

    #[test]
    fn optional_key_in_struct_roundtrips_both_formats() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Holder {
            key: Option<IdentityKey>,
            sig: Signature,
        }
        let id = Identity::generate();
        let value = Holder { key: Some(id.public()), sig: id.sign(b"x") };
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<Holder>(&json).unwrap(), value);
        let bytes = postcard::to_stdvec(&value).unwrap();
        assert_eq!(postcard::from_bytes::<Holder>(&bytes).unwrap(), value);
    }

    #[test]
    fn load_or_create_is_stable() {
        let dir = temp_dir("stable");
        let first = load_or_create(&dir).unwrap();
        let second = load_or_create(&dir).unwrap();
        assert_eq!(first.public(), second.public());
        let stored = std::fs::read_to_string(dir.join(IDENTITY_FILE)).unwrap();
        assert_eq!(stored, to_hex(&first.to_bytes()));
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(IDENTITY_FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn load_or_create_creates_missing_dir() {
        let dir = temp_dir("nested").join("a").join("b");
        let id = load_or_create(&dir).unwrap();
        assert_eq!(load_or_create(&dir).unwrap().public(), id.public());
    }

    #[test]
    fn load_or_create_picks_up_a_rewritten_file() {
        let dir = temp_dir("rewrite");
        let first = load_or_create(&dir).unwrap();
        let adopted = Identity::generate();
        std::fs::write(dir.join(IDENTITY_FILE), format!("{}\n", to_hex(&adopted.to_bytes()))).unwrap();
        let now = load_or_create(&dir).unwrap();
        assert_eq!(now.public(), adopted.public());
        assert_ne!(now.public(), first.public());
    }

    #[test]
    fn load_or_create_replaces_a_corrupt_file() {
        let dir = temp_dir("corrupt");
        for junk in [&b"not hex"[..], &[0xff, 0xfe, 0x00][..], &b"abcd"[..]] {
            std::fs::write(dir.join(IDENTITY_FILE), junk).unwrap();
            let id = load_or_create(&dir).unwrap();
            let stored = std::fs::read_to_string(dir.join(IDENTITY_FILE)).unwrap();
            assert_eq!(stored, to_hex(&id.to_bytes()));
        }
    }

    #[test]
    fn hex_helpers() {
        assert_eq!(to_hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
        assert_eq!(from_hex::<4>("000fa0ff"), Some([0x00, 0x0f, 0xa0, 0xff]));
        assert_eq!(from_hex::<4>("000FA0FF"), Some([0x00, 0x0f, 0xa0, 0xff]));
        assert_eq!(from_hex::<2>("+1ff"), None);
        assert_eq!(from_hex::<2>("é1f"), None);
    }
}
