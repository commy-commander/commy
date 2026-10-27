use crate::catchup::AppAccess;
use crate::state::App;
use commy_core::{Avatar, ServerState, UserId, UserProfile};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub const PROFILE_IMAGE_DIR: &str = "profile-images";
pub const SMALL_SERVER_MEMBERS: usize = 50;
pub const MAX_PROFILE_IMAGE_BYTES: usize = commy_view::pictures::MAX_PROFILE_IMAGE_BYTES;

pub fn image_blobs(profile: &UserProfile) -> Vec<String> {
    [profile.avatar.as_deref(), profile.banner.as_deref()]
        .into_iter()
        .flatten()
        .filter_map(|raw| match Avatar::parse(raw) {
            Some(Avatar::Image { blob }) => Some(blob),
            _ => None,
        })
        .collect()
}

pub fn people_to_keep<'a>(
    servers: impl IntoIterator<Item = &'a ServerState>,
    friends: &HashSet<UserId>,
    contacts: &'a HashMap<UserId, UserProfile>,
) -> HashMap<UserId, &'a UserProfile> {
    let mut kept: HashMap<UserId, &UserProfile> = HashMap::new();
    for state in servers {
        let small = state.server.members.len() <= SMALL_SERVER_MEMBERS;
        for profile in state.profiles.values() {
            if small || friends.contains(&profile.id) {
                kept.insert(profile.id, profile);
            }
        }
    }
    for profile in contacts.values() {
        kept.entry(profile.id).or_insert(profile);
    }
    kept
}

pub fn kept_blobs(app: &App) -> HashSet<String> {
    let friends = app.safety.lock().unwrap().friends.clone();
    let contacts = app.dms.lock().unwrap().contacts.clone();
    let me = app.profile.lock().unwrap().as_ref().map(|p| p.id);
    let servers = app.servers.lock().unwrap();
    people_to_keep(servers.values(), &friends, &contacts)
        .into_values()
        .filter(|profile| Some(profile.id) != me)
        .flat_map(image_blobs)
        .collect()
}

fn valid_blob(blob: &str) -> bool {
    Avatar::parse(&format!("blob:{blob}")).is_some()
}

fn cache_path(app: &App, blob: &str) -> Option<PathBuf> {
    if !valid_blob(blob) {
        return None;
    }
    app.data_dir().map(|d| d.join(PROFILE_IMAGE_DIR).join(format!("{blob}.img")))
}

pub fn cached(app: &App, blob: &str) -> Option<Vec<u8>> {
    commy_core::at_rest::read(&cache_path(app, blob)?).ok()
}

pub fn keep_if_wanted(app: &App, blob: &str, bytes: &[u8]) {
    if bytes.len() > MAX_PROFILE_IMAGE_BYTES || !commy_view::pictures::is_profile_picture(bytes) || !kept_blobs(app).contains(blob) {
        return;
    }
    let Some(path) = cache_path(app, blob) else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = commy_core::at_rest::write(&path, bytes) {
        tracing::debug!("profile image not kept on disk: {e}");
    }
}

pub fn prune(app: &App) {
    let Some(dir) = app.data_dir().map(|d| d.join(PROFILE_IMAGE_DIR)) else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let kept = kept_blobs(app);
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let wanted = name.strip_suffix(".img").is_some_and(|blob| kept.contains(blob));
        if !wanted {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

pub fn prefetch<A: AppAccess + Clone>(access: &A, profile: &UserProfile) {
    let Some(provider) = profile.node.clone() else { return };
    let kept = kept_blobs(access.app());
    let wanted: Vec<String> = image_blobs(profile)
        .into_iter()
        .filter(|blob| kept.contains(blob) && cached(access.app(), blob).is_none())
        .collect();
    if wanted.is_empty() {
        return;
    }
    let access = access.clone();
    crate::doorbell::spawn(async move {
        for blob in wanted {
            if let Err(e) = crate::profile_image_bytes(access.app(), &blob, Some(&provider)).await {
                tracing::debug!("profile image not fetched yet: {e}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use commy_core::Server;

    fn with_image(name: &str, blob: &str) -> UserProfile {
        let mut profile = UserProfile::new(name);
        profile.avatar = Some(format!("blob:{blob}"));
        profile
    }

    fn server_of(people: &[&UserProfile]) -> ServerState {
        let mut state = ServerState::new(Server::new("s", people[0].id));
        for person in people {
            state.server.members.retain(|m| m.user != person.id);
            state.server.members.push(commy_core::server::Member {
                user: person.id,
                nickname: None,
                roles: Vec::new(),
                joined_at: chrono::Utc::now(),
                timeout: None,
            });
            state.profiles.insert(person.id, (*person).clone());
        }
        state
    }

    fn crowd(size: usize) -> Vec<UserProfile> {
        (0..size).map(|i| with_image(&format!("p{i}"), &format!("blob{i}"))).collect()
    }

    #[test]
    fn avatars_and_banners_are_the_images_a_profile_needs() {
        let mut profile = with_image("a", "face");
        profile.banner = Some("blob:wide".to_string());
        assert_eq!(image_blobs(&profile), vec!["face".to_string(), "wide".to_string()]);

        profile.avatar = Some("emoji:🐸".to_string());
        profile.banner = None;
        assert!(image_blobs(&profile).is_empty());
    }

    #[test]
    fn everyone_in_a_small_server_is_kept_but_only_friends_in_a_big_one() {
        let small = crowd(3);
        let big = crowd(SMALL_SERVER_MEMBERS + 1);
        let small_server = server_of(&small.iter().collect::<Vec<_>>());
        let big_server = server_of(&big.iter().collect::<Vec<_>>());
        let friend = big[7].id;
        let contacts = HashMap::new();

        let kept = people_to_keep([&small_server, &big_server], &HashSet::from([friend]), &contacts);

        assert!(small.iter().all(|p| kept.contains_key(&p.id)));
        assert!(kept.contains_key(&friend));
        assert!(!kept.contains_key(&big[8].id));
        assert_eq!(kept.len(), small.len() + 1);
    }

    #[test]
    fn people_we_talk_to_directly_are_kept_without_a_shared_server() {
        let contact = with_image("c", "contact");
        let contacts = HashMap::from([(contact.id, contact.clone())]);
        let kept = people_to_keep(std::iter::empty(), &HashSet::new(), &contacts);
        assert_eq!(kept.get(&contact.id).map(|p| p.id), Some(contact.id));
    }

    #[test]
    fn a_kept_picture_survives_until_nobody_we_keep_shows_it() {
        let nanos = std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos();
        let dir = std::env::temp_dir().join("commy-profile-images-tests").join(format!("{nanos:x}"));
        std::fs::create_dir_all(&dir).unwrap();
        let app = App::default();
        app.data_dir.set(dir.clone()).unwrap();
        let png = [&[0x89, b'P', b'N', b'G'][..], &[0; 16]].concat();
        let friend = with_image("friend", "friendface");
        let stranger = with_image("stranger", "strangerface");
        app.dms.lock().unwrap().contacts.insert(friend.id, friend.clone());

        keep_if_wanted(&app, "friendface", &png);
        keep_if_wanted(&app, "strangerface", &png);
        keep_if_wanted(&app, "friendface", b"not a picture");
        assert_eq!(cached(&app, "friendface"), Some(png.clone()));
        assert_eq!(cached(&app, "strangerface"), None);

        prune(&app);
        assert_eq!(cached(&app, "friendface"), Some(png.clone()));

        app.dms.lock().unwrap().contacts.insert(friend.id, stranger);
        prune(&app);
        assert_eq!(cached(&app, "friendface"), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_real_blob_names_reach_the_disk() {
        assert!(valid_blob("abc123"));
        assert!(!valid_blob("../avatar"));
        assert!(!valid_blob(""));
        assert!(!valid_blob("a/b"));
    }
}
