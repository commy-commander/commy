use tauri::webview::{PermissionKind, PermissionResponse};

pub fn answer(kind: PermissionKind) -> PermissionResponse {
    let response = match kind {
        PermissionKind::Camera | PermissionKind::Microphone => PermissionResponse::Allow,
        _ => PermissionResponse::Default,
    };
    tracing::info!("the window asked for {kind:?}, answered {response:?}");
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_and_microphone_are_allowed() {
        assert_eq!(answer(PermissionKind::Camera), PermissionResponse::Allow);
        assert_eq!(answer(PermissionKind::Microphone), PermissionResponse::Allow);
    }

    #[test]
    fn other_requests_keep_the_webview_default() {
        assert_eq!(answer(PermissionKind::Geolocation), PermissionResponse::Default);
        assert_eq!(answer(PermissionKind::DisplayCapture), PermissionResponse::Default);
        assert_eq!(answer(PermissionKind::Notifications), PermissionResponse::Default);
    }
}
