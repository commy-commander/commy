use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ids::{ChannelId, DriveId, UserId};

pub const MAX_DRIVE_ITEMS: usize = 200;
pub const MAX_DRIVE_NAME: usize = 80;
pub const MAX_DRIVE_DEPTH: usize = 6;
pub const MAX_DRIVE_HOLDERS: usize = 16;
pub const MAX_DRIVE_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_MIME: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveFile {
    pub blob: String,
    pub size: u64,
    pub mime: String,
}

impl DriveFile {
    pub fn is_valid(&self) -> bool {
        is_hash(&self.blob)
            && (1..=MAX_DRIVE_FILE_BYTES).contains(&self.size)
            && !self.mime.is_empty()
            && self.mime.len() <= MAX_MIME
            && !self.mime.chars().any(|c| c.is_control() || c.is_whitespace())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveItem {
    pub id: DriveId,
    pub channel: ChannelId,
    pub parent: Option<DriveId>,
    pub name: String,
    pub file: Option<DriveFile>,
    pub by: UserId,
    pub at: i64,
    #[serde(default)]
    pub holders: BTreeMap<UserId, String>,
}

impl DriveItem {
    pub fn folder(channel: ChannelId, parent: Option<DriveId>, name: &str, by: UserId, at: i64) -> Option<Self> {
        let item = DriveItem { id: DriveId::new(), channel, parent, name: name.trim().to_string(), file: None, by, at, holders: BTreeMap::new() };
        item.is_valid().then_some(item)
    }

    pub fn file(channel: ChannelId, parent: Option<DriveId>, name: &str, file: DriveFile, by: UserId, node: &str, at: i64) -> Option<Self> {
        let holders = BTreeMap::from([(by, node.to_ascii_lowercase())]);
        let item = DriveItem { id: DriveId::new(), channel, parent, name: name.trim().to_string(), file: Some(file), by, at, holders };
        item.is_valid().then_some(item)
    }

    pub fn is_folder(&self) -> bool {
        self.file.is_none()
    }

    pub fn is_valid(&self) -> bool {
        let holders_ok = match &self.file {
            Some(file) => file.is_valid() && self.holders.len() <= 1 && self.holders.iter().all(|(user, node)| *user == self.by && is_hash(node)),
            None => self.holders.is_empty(),
        };
        is_name(&self.name) && holders_ok
    }
}

pub fn is_name(name: &str) -> bool {
    let count = name.chars().count();
    (1..=MAX_DRIVE_NAME).contains(&count)
        && name.trim() == name
        && name != "."
        && name != ".."
        && !name.chars().any(|c| c.is_control() || c == '/' || c == '\\')
}

pub fn clean_name(raw: &str) -> Option<String> {
    let flat: String = raw.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let flat = flat.trim();
    let name = if flat.chars().count() <= MAX_DRIVE_NAME {
        flat.to_string()
    } else {
        let ext = flat.rsplit_once('.').map(|(_, e)| e).filter(|e| !e.is_empty() && e.chars().count() <= 10).map(|e| format!(".{e}")).unwrap_or_default();
        let stem: String = flat.chars().take(MAX_DRIVE_NAME - ext.chars().count()).collect();
        format!("{}{ext}", stem.trim_end())
    };
    is_name(&name).then_some(name)
}

fn is_hash(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn find(items: &[DriveItem], id: DriveId) -> Option<&DriveItem> {
    items.iter().find(|i| i.id == id)
}

fn folder_in(items: &[DriveItem], channel: ChannelId, id: DriveId) -> Option<&DriveItem> {
    find(items, id).filter(|f| f.is_folder() && f.channel == channel)
}

pub fn depth(items: &[DriveItem], id: Option<DriveId>) -> usize {
    let mut depth = 0;
    let mut next = id;
    while let Some(id) = next {
        depth += 1;
        if depth > MAX_DRIVE_ITEMS {
            break;
        }
        next = find(items, id).and_then(|i| i.parent);
    }
    depth
}

fn height(items: &[DriveItem], id: DriveId) -> usize {
    let children: Vec<DriveId> = items.iter().filter(|i| i.parent == Some(id)).map(|i| i.id).collect();
    1 + children.into_iter().map(|c| height(items, c)).max().unwrap_or(0)
}

fn within(items: &[DriveItem], id: DriveId, ancestor: DriveId) -> bool {
    let mut next = Some(id);
    let mut steps = 0;
    while let Some(at) = next {
        if at == ancestor {
            return true;
        }
        steps += 1;
        if steps > MAX_DRIVE_ITEMS {
            return true;
        }
        next = find(items, at).and_then(|i| i.parent);
    }
    false
}

pub fn parent_fits(items: &[DriveItem], channel: ChannelId, parent: Option<DriveId>) -> bool {
    parent.is_none_or(|p| folder_in(items, channel, p).is_some())
}

pub fn add(items: &mut Vec<DriveItem>, item: DriveItem) -> bool {
    if !item.is_valid() || items.len() >= MAX_DRIVE_ITEMS || find(items, item.id).is_some() {
        return false;
    }
    if !parent_fits(items, item.channel, item.parent) || depth(items, item.parent) >= MAX_DRIVE_DEPTH {
        return false;
    }
    items.push(item);
    true
}

pub fn edit(items: &mut [DriveItem], id: DriveId, name: &str, parent: Option<DriveId>) -> bool {
    let Some(item) = find(items, id) else { return false };
    if !is_name(name) || (item.name == name && item.parent == parent) {
        return false;
    }
    if item.parent != parent {
        if !parent_fits(items, item.channel, parent) || parent.is_some_and(|p| within(items, p, id)) {
            return false;
        }
        if depth(items, parent) + height(items, id) > MAX_DRIVE_DEPTH {
            return false;
        }
    }
    let Some(item) = items.iter_mut().find(|i| i.id == id) else { return false };
    item.name = name.to_string();
    item.parent = parent;
    true
}

pub fn remove(items: &mut Vec<DriveItem>, id: DriveId) -> Vec<DriveId> {
    if find(items, id).is_none() {
        return Vec::new();
    }
    let gone: Vec<DriveId> = items.iter().filter(|i| within(items, i.id, id)).map(|i| i.id).collect();
    items.retain(|i| !gone.contains(&i.id));
    gone
}

pub fn clear_channel(items: &mut Vec<DriveItem>, channel: ChannelId) -> Vec<DriveId> {
    let gone: Vec<DriveId> = items.iter().filter(|i| i.channel == channel).map(|i| i.id).collect();
    items.retain(|i| i.channel != channel);
    gone
}

pub fn hold(items: &mut [DriveItem], id: DriveId, user: UserId, node: Option<&str>) -> bool {
    let Some(item) = items.iter_mut().find(|i| i.id == id && !i.is_folder()) else { return false };
    match node {
        Some(node) => {
            let node = node.to_ascii_lowercase();
            if !is_hash(&node) || item.holders.get(&user) == Some(&node) {
                return false;
            }
            if !item.holders.contains_key(&user) && item.holders.len() >= MAX_DRIVE_HOLDERS {
                return false;
            }
            item.holders.insert(user, node);
            true
        }
        None => item.holders.remove(&user).is_some(),
    }
}

pub fn forget_user(items: &mut [DriveItem], user: UserId) -> bool {
    let mut changed = false;
    for item in items.iter_mut() {
        changed |= item.holders.remove(&user).is_some();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    fn file(n: u8) -> DriveFile {
        DriveFile { blob: blob(n), size: 10, mime: "text/plain".into() }
    }

    #[test]
    fn names_are_short_and_cannot_hold_paths() {
        assert!(is_name("Maps"));
        assert!(is_name("season 2 (final).zip"));
        for bad in ["", " Maps", "a/b", "a\\b", "..", ".", "tab\there"] {
            assert!(!is_name(bad), "{bad:?}");
        }
        assert!(!is_name(&"a".repeat(MAX_DRIVE_NAME + 1)));
    }

    #[test]
    fn uploaded_file_names_are_tidied_to_fit() {
        assert_eq!(clean_name("  photo.png "), Some("photo.png".into()));
        assert_eq!(clean_name("a/b\\c.txt"), Some("a_b_c.txt".into()));
        let long = format!("{}.mkv", "x".repeat(200));
        let short = clean_name(&long).unwrap();
        assert_eq!(short.chars().count(), MAX_DRIVE_NAME);
        assert!(short.ends_with(".mkv"));
        assert_eq!(clean_name("  "), None);
        assert_eq!(clean_name(".."), None);
    }

    #[test]
    fn a_file_names_its_uploader_as_the_first_holder() {
        let (channel, by) = (ChannelId::new(), UserId::new());
        let node = blob(7);
        let item = DriveItem::file(channel, None, "notes.txt", file(1), by, &node.to_uppercase(), 1).unwrap();
        assert_eq!(item.holders.get(&by), Some(&node));
        let mut forged = item.clone();
        forged.holders.insert(UserId::new(), node.clone());
        assert!(!forged.is_valid());
        let mut huge = item.clone();
        huge.file.as_mut().unwrap().size = MAX_DRIVE_FILE_BYTES + 1;
        assert!(!huge.is_valid());
        let mut bad = item;
        bad.file.as_mut().unwrap().blob = "zz".into();
        assert!(!bad.is_valid());
    }

    #[test]
    fn folders_nest_in_their_own_channel_only() {
        let (channel, other, by) = (ChannelId::new(), ChannelId::new(), UserId::new());
        let mut items = Vec::new();
        let maps = DriveItem::folder(channel, None, "Maps", by, 1).unwrap();
        assert!(add(&mut items, maps.clone()));
        assert!(!add(&mut items, maps.clone()), "the same item is added once");
        assert!(!add(&mut items, DriveItem::folder(other, Some(maps.id), "Elsewhere", by, 2).unwrap()));
        assert!(!add(&mut items, DriveItem::folder(channel, Some(DriveId::new()), "Lost", by, 2).unwrap()));
        let doc = DriveItem::file(channel, Some(maps.id), "a.txt", file(1), by, &blob(9), 2).unwrap();
        assert!(add(&mut items, doc.clone()));
        assert!(!add(&mut items, DriveItem::folder(channel, Some(doc.id), "Inside a file", by, 3).unwrap()));
        let mut parent = None;
        let mut deep = Vec::new();
        for i in 0..MAX_DRIVE_DEPTH {
            let folder = DriveItem::folder(channel, parent, &format!("d{i}"), by, 3).unwrap();
            parent = Some(folder.id);
            assert!(add(&mut deep, folder));
        }
        assert!(!add(&mut deep, DriveItem::folder(channel, parent, "too deep", by, 4).unwrap()));
    }

    #[test]
    fn moves_never_make_a_loop() {
        let (channel, by) = (ChannelId::new(), UserId::new());
        let mut items = Vec::new();
        let a = DriveItem::folder(channel, None, "a", by, 1).unwrap();
        let b = DriveItem::folder(channel, Some(a.id), "b", by, 1).unwrap();
        add(&mut items, a.clone());
        add(&mut items, b.clone());
        assert!(!edit(&mut items, a.id, "a", Some(b.id)));
        assert!(!edit(&mut items, a.id, "a", Some(a.id)));
        assert!(!edit(&mut items, a.id, "a", None), "nothing changed");
        assert!(edit(&mut items, b.id, "bee", None));
        assert!(edit(&mut items, a.id, "a", Some(b.id)));
        assert_eq!(depth(&items, Some(a.id)), 2);
        assert!(!edit(&mut items, a.id, "bad/name", None));
    }

    #[test]
    fn removing_a_folder_takes_everything_in_it() {
        let (channel, by) = (ChannelId::new(), UserId::new());
        let mut items = Vec::new();
        let a = DriveItem::folder(channel, None, "a", by, 1).unwrap();
        let b = DriveItem::folder(channel, Some(a.id), "b", by, 1).unwrap();
        let c = DriveItem::file(channel, Some(b.id), "c", file(1), by, &blob(2), 1).unwrap();
        let d = DriveItem::file(channel, None, "d", file(3), by, &blob(2), 1).unwrap();
        for item in [&a, &b, &c, &d] {
            add(&mut items, item.clone());
        }
        let mut gone = remove(&mut items, a.id);
        gone.sort();
        let mut expected = vec![a.id, b.id, c.id];
        expected.sort();
        assert_eq!(gone, expected);
        assert_eq!(items.len(), 1);
        assert!(remove(&mut items, a.id).is_empty());
        assert_eq!(clear_channel(&mut items, channel), vec![d.id]);
    }

    #[test]
    fn holders_come_and_go_up_to_a_limit() {
        let (channel, by) = (ChannelId::new(), UserId::new());
        let mut items = Vec::new();
        let doc = DriveItem::file(channel, None, "doc", file(1), by, &blob(2), 1).unwrap();
        let folder = DriveItem::folder(channel, None, "f", by, 1).unwrap();
        add(&mut items, doc.clone());
        add(&mut items, folder.clone());
        assert!(!hold(&mut items, folder.id, by, Some(&blob(2))));
        assert!(!hold(&mut items, doc.id, by, Some(&blob(2))), "already held from that device");
        assert!(hold(&mut items, doc.id, by, Some(&blob(3))));
        assert!(!hold(&mut items, doc.id, by, Some("short")));
        for _ in 1..MAX_DRIVE_HOLDERS {
            assert!(hold(&mut items, doc.id, UserId::new(), Some(&blob(4))));
        }
        let late = UserId::new();
        assert!(!hold(&mut items, doc.id, late, Some(&blob(5))));
        assert!(hold(&mut items, doc.id, by, None));
        assert!(hold(&mut items, doc.id, late, Some(&blob(5))));
        assert!(forget_user(&mut items, late));
        assert!(!forget_user(&mut items, late));
    }

    #[test]
    fn a_server_holds_a_bounded_number_of_items() {
        let (channel, by) = (ChannelId::new(), UserId::new());
        let mut items = Vec::new();
        for i in 0..MAX_DRIVE_ITEMS {
            assert!(add(&mut items, DriveItem::folder(channel, None, &format!("f{i}"), by, 1).unwrap()));
        }
        assert!(!add(&mut items, DriveItem::folder(channel, None, "one more", by, 1).unwrap()));
    }
}
