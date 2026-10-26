use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_DEVICE_INFO_TYPE,
    DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND, LUID};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};

const ADVANCED_COLOR_ENABLED: u32 = 0b10;
const WHITE_LEVEL_PER_SCRGB_UNIT: f32 = 1000.0;

pub fn monitor_of_window(hwnd: u32) -> u32 {
    let monitor = unsafe { MonitorFromWindow(HWND(hwnd as usize as *mut _), MONITOR_DEFAULTTONEAREST) };
    monitor.0 as usize as u32
}

pub fn hdr_white_level(hmonitor: u32) -> Option<f32> {
    let device = gdi_device_name(HMONITOR(hmonitor as usize as *mut _))?;
    let path = active_paths()?.into_iter().find(|path| {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        let (adapter, source_id) = (path.sourceInfo.adapterId, path.sourceInfo.id);
        device_info(&mut source, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, adapter, source_id)
            && source.viewGdiDeviceName == device
    })?;
    let (adapter, target) = (path.targetInfo.adapterId, path.targetInfo.id);
    let mut color = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO::default();
    if !device_info(&mut color, DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO, adapter, target) {
        return None;
    }
    if unsafe { color.Anonymous.value } & ADVANCED_COLOR_ENABLED == 0 {
        return None;
    }
    let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL::default();
    if !device_info(&mut white, DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, adapter, target) {
        return Some(1.0);
    }
    Some((white.SDRWhiteLevel as f32 / WHITE_LEVEL_PER_SCRGB_UNIT).max(1.0))
}

fn gdi_device_name(monitor: HMONITOR) -> Option<[u16; 32]> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    let found = unsafe { GetMonitorInfoW(monitor, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) };
    found.as_bool().then_some(info.szDevice)
}

fn active_paths() -> Option<Vec<DISPLAYCONFIG_PATH_INFO>> {
    let (mut path_count, mut mode_count) = (0u32, 0u32);
    let sized = unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count) };
    if sized != ERROR_SUCCESS {
        return None;
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    let result = unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
    };
    if result != ERROR_SUCCESS {
        return None;
    }
    paths.truncate(path_count as usize);
    Some(paths)
}

fn device_info<T>(packet: &mut T, kind: DISPLAYCONFIG_DEVICE_INFO_TYPE, adapter: LUID, id: u32) -> bool {
    let header = packet as *mut T as *mut DISPLAYCONFIG_DEVICE_INFO_HEADER;
    unsafe {
        (*header).r#type = kind;
        (*header).size = size_of::<T>() as u32;
        (*header).adapterId = adapter;
        (*header).id = id;
        DisplayConfigGetDeviceInfo(header) == 0
    }
}
