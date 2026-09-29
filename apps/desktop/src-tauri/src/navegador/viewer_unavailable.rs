//! The browser when the `navegador` feature is off: nothing to drive.

use tauri::{AppHandle, Url};

use super::bounds::Bounds;
use super::{ViewerState, UNAVAILABLE};

pub const AVAILABLE: bool = false;

pub fn open(_app: &AppHandle, _url: Url, _bounds: Bounds) -> Result<ViewerState, String> {
    Err(UNAVAILABLE.to_string())
}

pub fn navigate(_app: &AppHandle, _url: Url) -> Result<ViewerState, String> {
    Err(UNAVAILABLE.to_string())
}

pub fn back(_app: &AppHandle) -> Result<(), String> {
    Err(UNAVAILABLE.to_string())
}

pub fn forward(_app: &AppHandle) -> Result<(), String> {
    Err(UNAVAILABLE.to_string())
}

pub fn reload(_app: &AppHandle) -> Result<(), String> {
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

pub fn state(_app: &AppHandle) -> Result<ViewerState, String> {
    Err(UNAVAILABLE.to_string())
}

/// Nothing to close.
pub fn shutdown(_app: &AppHandle) {}
