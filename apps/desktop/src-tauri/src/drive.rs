#[cfg(desktop)]
use commy_core::drive::{DriveFile, MAX_DRIVE_FILE_BYTES};
use commy_core::ids::DriveId;
use commy_core::{ChannelId, ServerId};
use commy_view::drive::DriveView;
#[cfg(desktop)]
use tauri::Emitter;

use crate::state::App;
use crate::{apply_and_broadcast, current_profile, err_to_string, managed, CmdResult};

#[cfg(mobile)]
const DESKTOP_ONLY: &str = "Drives work in the desktop app";
#[cfg(desktop)]
const TOO_BIG: &str = "files larger than 4 GB can't go in a drive";

fn ids(server_id: &str) -> CmdResult<ServerId> {
    server_id.parse().map_err(err_to_string)
}

fn item_id(raw: &str) -> CmdResult<DriveId> {
    raw.parse().map_err(|_| "unknown drive item".to_string())
}

fn parent_id(raw: Option<&str>) -> CmdResult<Option<DriveId>> {
    raw.filter(|r| !r.is_empty()).map(item_id).transpose()
}

async fn own_node(app: &App) -> Option<String> {
    app.net.lock().await.as_ref().map(|n| n.node_id_hex())
}

#[tauri::command]
pub async fn drive_view(app: tauri::State<'_, App>, server_id: String, channel_id: String) -> CmdResult<DriveView> {
    let sid = ids(&server_id)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let me = current_profile(&app)?;
    let node = own_node(&app).await;
    managed(&app, sid, me.id, |state, _| commy_view::drive::view(state, cid, me.id, node.as_deref()))
}

#[tauri::command]
pub async fn drive_new_folder(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    parent_id: Option<String>,
    name: String,
) -> CmdResult<()> {
    let sid = ids(&server_id)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let parent = self::parent_id(parent_id.as_deref())?;
    let me = current_profile(&app)?;
    let now = chrono::Utc::now().timestamp_millis();
    let event = managed(&app, sid, me.id, |state, _| commy_view::drive::new_folder(state, me.id, cid, parent, &name, now))?;
    apply_and_broadcast(&app, &handle, sid, event).await
}

#[tauri::command]
pub async fn drive_rename(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    item_id: String,
    name: String,
    parent_id: Option<String>,
) -> CmdResult<()> {
    let sid = ids(&server_id)?;
    let item = self::item_id(&item_id)?;
    let parent = self::parent_id(parent_id.as_deref())?;
    let me = current_profile(&app)?;
    let event = managed(&app, sid, me.id, |state, _| commy_view::drive::edit(state, me.id, item, &name, parent))?;
    apply_and_broadcast(&app, &handle, sid, event).await
}

fn blob_of(app: &App, sid: ServerId, item: DriveId) -> Option<String> {
    let servers = app.servers.lock().unwrap();
    let state = servers.get(&sid)?;
    commy_core::drive::find(&state.server.drive, item).and_then(|i| i.file.as_ref()).map(|f| f.blob.clone())
}

fn kept_elsewhere(app: &App, sid: ServerId, item: DriveId, blob: &str, me: commy_core::UserId) -> bool {
    let servers = app.servers.lock().unwrap();
    servers.iter().any(|(id, state)| {
        state.server.drive.iter().any(|i| {
            (*id != sid || i.id != item) && i.holders.contains_key(&me) && i.file.as_ref().is_some_and(|f| f.blob == blob)
        })
    })
}

#[cfg(desktop)]
async fn stop_sharing(app: &App, blob: &str) {
    let net = app.net.lock().await;
    let Some(node) = net.as_ref() else { return };
    if let Err(e) = node.forget_file(blob).await {
        tracing::info!("drive file is still shared: {e:#}");
    }
}

#[cfg(mobile)]
async fn stop_sharing(_app: &App, _blob: &str) {}

#[tauri::command]
pub async fn drive_remove(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, item_id: String) -> CmdResult<()> {
    let sid = ids(&server_id)?;
    let item = self::item_id(&item_id)?;
    let me = current_profile(&app)?;
    let mine: Vec<(DriveId, String)> = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        let drive = &state.server.drive;
        let inside = |i: &commy_core::drive::DriveItem| {
            let mut at = Some(i.id);
            while let Some(id) = at {
                if id == item {
                    return true;
                }
                at = commy_core::drive::find(drive, id).and_then(|p| p.parent);
            }
            false
        };
        drive.iter().filter(|i| inside(i) && i.holders.contains_key(&me.id)).filter_map(|i| i.file.as_ref().map(|f| (i.id, f.blob.clone()))).collect()
    };
    let event = managed(&app, sid, me.id, |state, _| commy_view::drive::remove(state, me.id, item))?;
    apply_and_broadcast(&app, &handle, sid, event).await?;
    for (id, blob) in mine {
        if !kept_elsewhere(&app, sid, id, &blob, me.id) {
            stop_sharing(&app, &blob).await;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn drive_stop_keeping(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, item_id: String) -> CmdResult<()> {
    let sid = ids(&server_id)?;
    let item = self::item_id(&item_id)?;
    let me = current_profile(&app)?;
    let blob = blob_of(&app, sid, item).ok_or(commy_view::drive::NO_SUCH_ITEM)?;
    let event = managed(&app, sid, me.id, |state, _| commy_view::drive::keep(state, me.id, item, None))?;
    apply_and_broadcast(&app, &handle, sid, event).await?;
    if !kept_elsewhere(&app, sid, item, &blob, me.id) {
        stop_sharing(&app, &blob).await;
    }
    Ok(())
}

#[cfg(desktop)]
#[tauri::command]
pub async fn drive_upload(
    app: tauri::State<'_, App>,
    handle: tauri::AppHandle,
    server_id: String,
    channel_id: String,
    parent_id: Option<String>,
) -> CmdResult<bool> {
    let sid = ids(&server_id)?;
    let cid: ChannelId = channel_id.parse().map_err(err_to_string)?;
    let parent = self::parent_id(parent_id.as_deref())?;
    let me = current_profile(&app)?;
    managed(&app, sid, me.id, |state, _| commy_view::drive::view(state, cid, me.id, None).and_then(|v| if v.can_upload { Ok(()) } else { Err("you do not have permission to upload files here".into()) }))?;
    let Some(picked) = crate::picker::pick_attachment(&handle, "Add a file to the drive", None, 0, MAX_DRIVE_FILE_BYTES, TOO_BIG).await? else {
        return Ok(false);
    };
    let crate::picker::Contents::OnDisk { path, .. } = picked.contents else {
        return Err("empty files can't go in a drive".into());
    };
    let name = commy_core::drive::clean_name(&picked.name).ok_or(commy_view::drive::BAD_NAME)?;
    let mime = mime_guess::from_path(&name).first_raw().unwrap_or("application/octet-stream").to_string();
    let (blob, size, node) = {
        let net = app.net.lock().await;
        let node = net.as_ref().ok_or("network is not ready yet")?;
        let (blob, size) = node.share_file(&path).await.map_err(err_to_string)?;
        (blob, size, node.node_id_hex())
    };
    let file = DriveFile { blob, size, mime };
    let now = chrono::Utc::now().timestamp_millis();
    let event = managed(&app, sid, me.id, |state, _| commy_view::drive::add_file(state, me.id, cid, parent, &name, file, &node, now))?;
    apply_and_broadcast(&app, &handle, sid, event).await?;
    Ok(true)
}

#[cfg(mobile)]
#[tauri::command]
pub async fn drive_upload(server_id: String, channel_id: String, parent_id: Option<String>) -> CmdResult<bool> {
    let _ = (server_id, channel_id, parent_id);
    Err(DESKTOP_ONLY.into())
}

#[cfg(desktop)]
#[derive(Clone, serde::Serialize)]
struct DriveProgress<'a> {
    item: &'a str,
    done: u64,
    total: u64,
}

#[cfg(desktop)]
#[tauri::command]
pub async fn drive_download(app: tauri::State<'_, App>, handle: tauri::AppHandle, server_id: String, item_id: String) -> CmdResult<Option<String>> {
    let sid = ids(&server_id)?;
    let item = self::item_id(&item_id)?;
    let me = current_profile(&app)?;
    let (name, blob, holders) = {
        let servers = app.servers.lock().unwrap();
        let state = servers.get(&sid).ok_or("unknown server")?;
        let found = commy_core::drive::find(&state.server.drive, item).ok_or(commy_view::drive::NO_SUCH_ITEM)?;
        let file = found.file.as_ref().ok_or("that is a folder")?;
        if !state.server.can_view(found.channel, me.id) {
            return Err(commy_view::drive::NO_SUCH_ITEM.into());
        }
        let view = commy_view::drive::view(state, found.channel, me.id, None)?;
        let sources = view.items.into_iter().find(|i| i.id == item_id).map(|i| i.sources).unwrap_or_default();
        (found.name.clone(), file.blob.clone(), sources)
    };
    let Some(target) = crate::picker::save_path(&handle, &crate::safe_file_name(&name)).await else {
        return Ok(None);
    };
    let members = crate::member_nodes(&app, sid);
    let (fetcher, sources, node) = {
        let net = app.net.lock().await;
        let node = net.as_ref().ok_or("network is not ready yet")?;
        let mut sources = holders;
        for neighbor in commy_net::node::blob_sources("", node.neighbors(sid)) {
            if !sources.contains(&neighbor) {
                sources.push(neighbor);
            }
        }
        (node.blob_fetcher(), sources, node.node_id_hex())
    };
    let report = |done, total| {
        let _ = handle.emit("drive-progress", DriveProgress { item: &item_id, done, total });
    };
    fetcher.download_among(&blob, sources.iter().map(String::as_str), &members, MAX_DRIVE_FILE_BYTES, &target, report).await.map_err(err_to_string)?;
    let shared = {
        let net = app.net.lock().await;
        match net.as_ref() {
            Some(node) => node.shares_file(&blob).await,
            None => false,
        }
    };
    if shared {
        let event = managed(&app, sid, me.id, |state, _| commy_view::drive::keep(state, me.id, item, Some(&node)));
        match event {
            Ok(event) => {
                if let Err(e) = apply_and_broadcast(&app, &handle, sid, event).await {
                    tracing::info!("could not say we keep a drive file: {e}");
                }
            }
            Err(e) => tracing::info!("not keeping a drive file: {e}"),
        }
    }
    Ok(Some(target.display().to_string()))
}

#[cfg(mobile)]
#[tauri::command]
pub async fn drive_download(server_id: String, item_id: String) -> CmdResult<Option<String>> {
    let _ = (server_id, item_id);
    Err(DESKTOP_ONLY.into())
}
