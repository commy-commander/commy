use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const TRAILER_MARK: [u8; 4] = [0xc0, 0x6d, 0x77, 0x01];

type Entry = (u16, u32, Vec<u8>);

#[derive(Default)]
struct Writing {
    seen: HashMap<u16, u32>,
    entries: Vec<Entry>,
}

#[derive(Default)]
struct Reading {
    seen: HashMap<u16, u32>,
    entries: HashMap<(u16, u32), Vec<u8>>,
}

thread_local! {
    static FIRST_LAYOUT: Cell<bool> = const { Cell::new(false) };
    static WRITING: RefCell<Option<Writing>> = const { RefCell::new(None) };
    static READING: RefCell<Option<Reading>> = const { RefCell::new(None) };
}

struct Layout {
    was_first: bool,
    writing: Option<Writing>,
    reading: Option<Reading>,
}

impl Layout {
    fn writing() -> Self {
        Self::enter(true, Some(Writing::default()), None)
    }

    fn reading(entries: Option<HashMap<(u16, u32), Vec<u8>>>) -> Self {
        Self::enter(true, None, entries.map(|entries| Reading { seen: HashMap::new(), entries }))
    }

    fn full() -> Self {
        Self::enter(false, None, None)
    }

    fn enter(first: bool, writing: Option<Writing>, reading: Option<Reading>) -> Self {
        Self {
            was_first: FIRST_LAYOUT.replace(first),
            writing: WRITING.replace(writing),
            reading: READING.replace(reading),
        }
    }

    fn written(&self) -> Vec<Entry> {
        WRITING.with_borrow_mut(|w| w.take().map(|w| w.entries).unwrap_or_default())
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        FIRST_LAYOUT.set(self.was_first);
        WRITING.set(self.writing.take());
        READING.set(self.reading.take());
    }
}

pub fn encode<T: Serialize + ?Sized>(value: &T) -> postcard::Result<Vec<u8>> {
    let layout = Layout::writing();
    let mut bytes = postcard::to_stdvec(value)?;
    let entries = layout.written();
    drop(layout);
    if !entries.is_empty() {
        bytes.extend_from_slice(&TRAILER_MARK);
        bytes.extend(postcard::to_stdvec(&entries)?);
    }
    Ok(bytes)
}

pub fn first_layout<T: Serialize + ?Sized>(value: &T) -> postcard::Result<Vec<u8>> {
    let _layout = Layout::writing();
    postcard::to_stdvec(value)
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> postcard::Result<T> {
    if let Some(value) = decode_with_trailer(bytes) {
        return Ok(value);
    }
    let _layout = Layout::full();
    postcard::from_bytes(bytes)
}

pub fn read_as_first_layout<T: DeserializeOwned>(bytes: &[u8]) -> postcard::Result<T> {
    let _layout = Layout::reading(None);
    postcard::from_bytes(bytes)
}

fn decode_with_trailer<T: DeserializeOwned>(bytes: &[u8]) -> Option<T> {
    let (value, rest) = {
        let _layout = Layout::reading(None);
        postcard::take_from_bytes::<T>(bytes).ok()?
    };
    if rest.is_empty() {
        return Some(value);
    }
    let (entries, []) = postcard::take_from_bytes::<Vec<Entry>>(rest.strip_prefix(&TRAILER_MARK)?).ok()? else {
        return None;
    };
    let entries = entries.into_iter().map(|(key, nth, bytes)| ((key, nth), bytes)).collect();
    let _layout = Layout::reading(Some(entries));
    postcard::take_from_bytes::<T>(bytes).ok().map(|(value, _)| value)
}

fn next_visit(seen: &mut HashMap<u16, u32>, key: u16) -> u32 {
    let nth = seen.entry(key).or_default();
    *nth += 1;
    *nth - 1
}

fn write<T, S>(key: u16, value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    T: Serialize + Default + PartialEq,
    S: Serializer,
{
    if !FIRST_LAYOUT.get() {
        return value.serialize(serializer);
    }
    let nth = WRITING.with_borrow_mut(|w| w.as_mut().map(|w| next_visit(&mut w.seen, key)));
    if let Some(nth) = nth.filter(|_| *value != T::default()) {
        let bytes = encode(value).map_err(serde::ser::Error::custom)?;
        WRITING.with_borrow_mut(|w| w.as_mut().map(|w| w.entries.push((key, nth, bytes))));
    }
    serializer.serialize_unit()
}

fn read<'de, T, D>(key: u16, deserializer: D) -> Result<T, D::Error>
where
    T: DeserializeOwned + Default,
    D: Deserializer<'de>,
{
    if !FIRST_LAYOUT.get() {
        return T::deserialize(deserializer);
    }
    <()>::deserialize(deserializer)?;
    let bytes = READING.with_borrow_mut(|r| {
        let r = r.as_mut()?;
        let nth = next_visit(&mut r.seen, key);
        r.entries.remove(&(key, nth))
    });
    Ok(bytes.and_then(|bytes| decode(&bytes).ok()).unwrap_or_default())
}

macro_rules! later_fields {
    ($($name:ident = $key:literal),* $(,)?) => {
        pub mod later {
            pub const KEYS: &[(&str, u16)] = &[$((stringify!($name), $key)),*];
            $(
                pub mod $name {
                    pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
                    where
                        T: serde::Serialize + Default + PartialEq,
                        S: serde::Serializer,
                    {
                        super::super::write($key, value, serializer)
                    }

                    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
                    where
                        T: serde::de::DeserializeOwned + Default,
                        D: serde::Deserializer<'de>,
                    {
                        super::super::read($key, deserializer)
                    }
                }
            )*
        }
    };
}

later_fields! {
    profile_home = 1,
    server_sync_host = 2,
    server_blocked_homes = 3,
    server_require_home = 4,
    sync_response_full_sig = 5,
    server_day_keys = 6,
    server_day_keys_ready = 7,
    snapshot_day_chain = 8,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Before {
        name: String,
        count: u32,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct After {
        name: String,
        count: u32,
        #[serde(default, with = "later::profile_home")]
        home: Option<String>,
        #[serde(default, with = "later::server_blocked_homes")]
        blocked: Vec<String>,
    }

    #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
    struct Nested {
        #[serde(default, with = "later::server_blocked_homes")]
        inner: Vec<String>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Outer {
        rooms: Vec<After>,
        #[serde(default, with = "later::profile_home")]
        nested: Option<Nested>,
    }

    fn after(home: Option<&str>) -> After {
        After { name: "lobby".into(), count: 3, home: home.map(str::to_string), blocked: Vec::new() }
    }

    #[test]
    fn values_without_later_fields_keep_the_first_layout_both_ways() {
        let old = postcard::to_stdvec(&(Before { name: "lobby".into(), count: 3 }, 9u8)).unwrap();
        assert_eq!(encode(&(after(None), 9u8)).unwrap(), old);
        assert_eq!(decode::<(After, u8)>(&old).unwrap(), (after(None), 9));
        let back: (Before, u8) = postcard::from_bytes(&encode(&(after(None), 9u8)).unwrap()).unwrap();
        assert_eq!(back.1, 9);
    }

    #[test]
    fn older_builds_read_values_with_later_fields_set() {
        let value = (vec![after(Some("https://home.example")), after(None), after(Some("https://b.example"))], 9u8);
        let bytes = encode(&value).unwrap();
        let older: (Vec<Before>, u8) = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(older.0.len(), 3);
        assert_eq!(older.0[2].name, "lobby");
        assert_eq!(older.1, 9);
        let first = first_layout(&value).unwrap();
        assert_eq!(&bytes[..first.len()], &first[..]);
        assert_eq!(bytes[first.len()..first.len() + 4], TRAILER_MARK);
    }

    #[test]
    fn values_with_later_fields_come_back_whole() {
        let value = (vec![after(Some("https://home.example")), after(None), after(Some("https://b.example"))], 9u8);
        assert_eq!(decode::<(Vec<After>, u8)>(&encode(&value).unwrap()).unwrap(), value);
        let mut blocked = after(None);
        blocked.blocked = vec!["bad.example".into()];
        let value = Outer { rooms: vec![blocked, after(Some("x"))], nested: Some(Nested { inner: vec!["deep".into()] }) };
        assert_eq!(decode::<Outer>(&encode(&value).unwrap()).unwrap(), value);
    }

    #[test]
    fn later_fields_this_build_does_not_know_are_skipped() {
        let value = Outer { rooms: vec![after(Some("x"))], nested: Some(Nested { inner: vec!["deep".into()] }) };
        let bytes = encode(&value).unwrap();
        #[derive(Debug, Deserialize)]
        struct Older {
            rooms: Vec<After>,
        }
        let older: Older = decode(&bytes).unwrap();
        assert_eq!(older.rooms, value.rooms);
    }

    #[test]
    fn full_layout_bytes_from_release_candidates_still_read() {
        let value = (after(Some("https://home.example")), 9u8);
        let full = postcard::to_stdvec(&value).unwrap();
        assert_eq!(decode::<(After, u8)>(&full).unwrap(), value);
        let value = (after(None), 9u8);
        assert_eq!(decode::<(After, u8)>(&postcard::to_stdvec(&value).unwrap()).unwrap(), value);
    }

    #[test]
    fn reading_as_the_first_layout_drops_later_fields() {
        let value = (after(Some("https://home.example")), 9u8);
        let read: (After, u8) = read_as_first_layout(&encode(&value).unwrap()).unwrap();
        assert_eq!(read, (after(None), 9));
    }

    #[test]
    fn json_always_carries_every_field() {
        let value = after(Some("https://home.example"));
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<After>(&json).unwrap(), value);
        assert_eq!(serde_json::from_str::<After>(r#"{"name":"lobby","count":3}"#).unwrap(), after(None));
    }

    #[test]
    fn every_later_field_has_its_own_key() {
        let mut keys: Vec<u16> = later::KEYS.iter().map(|(_, key)| *key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), later::KEYS.len());
    }

    #[test]
    fn the_layout_switch_does_not_leak_out_of_a_call() {
        let _ = encode(&(after(Some("x")), 1u8));
        let _ = decode::<(After, u8)>(&[0xff]);
        assert!(!FIRST_LAYOUT.get());
        assert!(WRITING.with_borrow(|w| w.is_none()));
        assert!(READING.with_borrow(|r| r.is_none()));
        assert_eq!(postcard::to_stdvec(&after(Some("x"))).unwrap(), {
            let mut full = postcard::to_stdvec(&Before { name: "lobby".into(), count: 3 }).unwrap();
            full.extend(postcard::to_stdvec(&Some("x")).unwrap());
            full.extend(postcard::to_stdvec(&Vec::<String>::new()).unwrap());
            full
        });
    }
}
