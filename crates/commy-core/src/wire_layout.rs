use std::cell::Cell;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

thread_local! {
    static FIRST_LAYOUT: Cell<bool> = const { Cell::new(false) };
    static LATER_FIELD_DROPPED: Cell<bool> = const { Cell::new(false) };
}

struct FirstLayout {
    was_on: bool,
    had_dropped: bool,
}

impl FirstLayout {
    fn on() -> Self {
        Self { was_on: FIRST_LAYOUT.replace(true), had_dropped: LATER_FIELD_DROPPED.replace(false) }
    }

    fn dropped_a_later_field(&self) -> bool {
        LATER_FIELD_DROPPED.get()
    }
}

impl Drop for FirstLayout {
    fn drop(&mut self) {
        FIRST_LAYOUT.set(self.was_on);
        LATER_FIELD_DROPPED.set(self.had_dropped);
    }
}

pub fn encode<T: Serialize + ?Sized>(value: &T) -> postcard::Result<Vec<u8>> {
    let first = {
        let layout = FirstLayout::on();
        let bytes = postcard::to_stdvec(value)?;
        (!layout.dropped_a_later_field()).then_some(bytes)
    };
    match first {
        Some(bytes) => Ok(bytes),
        None => postcard::to_stdvec(value),
    }
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> postcard::Result<T> {
    if let Some(value) = decode_first_exactly(bytes) {
        return Ok(value);
    }
    postcard::from_bytes(bytes)
}

fn decode_first_exactly<T: DeserializeOwned>(bytes: &[u8]) -> Option<T> {
    let _layout = FirstLayout::on();
    match postcard::take_from_bytes::<T>(bytes) {
        Ok((value, [])) => Some(value),
        _ => None,
    }
}

pub mod later {
    use super::*;

    pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: Serialize + Default + PartialEq,
        S: Serializer,
    {
        if !FIRST_LAYOUT.get() {
            return value.serialize(serializer);
        }
        if *value != T::default() {
            LATER_FIELD_DROPPED.set(true);
        }
        serializer.serialize_unit()
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
    where
        T: Deserialize<'de> + Default,
        D: Deserializer<'de>,
    {
        if !FIRST_LAYOUT.get() {
            return T::deserialize(deserializer);
        }
        <()>::deserialize(deserializer)?;
        Ok(T::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Before {
        name: String,
        count: u32,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct After {
        name: String,
        count: u32,
        #[serde(default, with = "later")]
        home: Option<String>,
        #[serde(default, with = "later")]
        blocked: Vec<String>,
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
    fn values_with_later_fields_use_the_full_layout() {
        let value = (after(Some("https://home.example")), 9u8);
        let bytes = encode(&value).unwrap();
        assert_eq!(bytes, postcard::to_stdvec(&value).unwrap());
        assert_eq!(decode::<(After, u8)>(&bytes).unwrap(), value);
    }

    #[test]
    fn full_layout_bytes_with_empty_later_fields_still_read() {
        let value = (after(None), 9u8);
        let full = postcard::to_stdvec(&value).unwrap();
        assert_eq!(decode::<(After, u8)>(&full).unwrap(), value);
    }

    #[test]
    fn json_always_carries_every_field() {
        let value = after(Some("https://home.example"));
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<After>(&json).unwrap(), value);
        assert_eq!(serde_json::from_str::<After>(r#"{"name":"lobby","count":3}"#).unwrap(), after(None));
    }

    #[test]
    fn the_layout_switch_does_not_leak_out_of_a_call() {
        let _ = encode(&(after(Some("x")), 1u8));
        let _ = decode::<(After, u8)>(&[0xff]);
        assert!(!FIRST_LAYOUT.get());
        assert_eq!(postcard::to_stdvec(&after(Some("x"))).unwrap(), {
            let mut full = postcard::to_stdvec(&Before { name: "lobby".into(), count: 3 }).unwrap();
            full.extend(postcard::to_stdvec(&Some("x")).unwrap());
            full.extend(postcard::to_stdvec(&Vec::<String>::new()).unwrap());
            full
        });
    }
}
