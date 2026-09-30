//! The browser when the `navegador` feature is off: nothing to drive.

use tauri::{AppHandle, Url};

use super::bounds::Bounds;
use super::capture::{CaptureDraft, CaptureError, CaptureKind};
use super::tabs::BrowserState;
use super::UNAVAILABLE;

pub const AVAILABLE: bool = false;

pub fn open(_app: &AppHandle, _url: Url, _bounds: Bounds) -> Result<BrowserState, String> {
    Err(UNAVAILABLE.to_string())
}

pub fn navigate(_app: &AppHandle, _tab: u32, _url: Url) -> Result<BrowserState, String> {
    Err(UNAVAILABLE.to_string())
}

pub fn new_tab(_app: &AppHandle) -> Result<BrowserState, String> {
    Err(UNAVAILABLE.to_string())
}

pub fn activate_tab(_app: &AppHandle, _tab: u32) -> Result<BrowserState, String> {
    Err(UNAVAILABLE.to_string())
}

pub fn close_tab(_app: &AppHandle, _tab: u32) -> Result<BrowserState, String> {
    Err(UNAVAILABLE.to_string())
}

pub fn back(_app: &AppHandle, _tab: u32) -> Result<(), String> {
    Err(UNAVAILABLE.to_string())
}

pub fn forward(_app: &AppHandle, _tab: u32) -> Result<(), String> {
    Err(UNAVAILABLE.to_string())
}

pub fn reload(_app: &AppHandle, _tab: u32) -> Result<(), String> {
    Err(UNAVAILABLE.to_string())
}

pub fn set_bounds(_app: &AppHandle, _bounds: Bounds) -> Result<(), String> {
    Err(UNAVAILABLE.to_string())
}

pub fn set_visible(_app: &AppHandle, _visible: bool) -> Result<(), String> {
    Err(UNAVAILABLE.to_string())
}

pub fn close(_app: &AppHandle) -> Result<(), String> {
    Err(UNAVAILABLE.to_string())
}

pub fn state(_app: &AppHandle) -> Result<BrowserState, String> {
    Err(UNAVAILABLE.to_string())
}

pub async fn capture(
    _app: &AppHandle,
    _tab: u32,
    _kind: CaptureKind,
) -> Result<CaptureDraft, CaptureError> {
    Err(CaptureError::with_detail(
        super::capture::code::NOT_OPEN,
        UNAVAILABLE,
    ))
}

/// Nothing to close.
pub fn set_download_dir(_app: &AppHandle, _dir: Option<std::path::PathBuf>) {}

pub fn shutdown(_app: &AppHandle) {}
