//! A clear notice when the Microsoft Edge WebView2 Runtime is missing.
//!
//! The MSI and NSIS installers install WebView2 themselves; an MSIX package
//! (Microsoft Store) cannot run an installer. Without the runtime, building
//! the Tauri app fails and the process ended with no window and no message.
//! [`ensure_runtime`] runs first in `run()`: it asks the WebView2 loader for
//! the installed version and, when there is none, explains what is missing,
//! offers Microsoft's download, and exits.

use webview2_com::Microsoft::Web::WebView2::Win32::GetAvailableCoreWebView2BrowserVersionString;
use windows::core::{HSTRING, PCWSTR, PWSTR};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_ICONERROR, MB_SETFOREGROUND, MB_YESNO, SW_SHOWNORMAL,
};

/// Microsoft's Evergreen bootstrapper for the WebView2 Runtime.
pub(crate) const DOWNLOAD_URL: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

const TITLE: &str = "EntropIA";

/// The notice, in Spanish first and English after: the app's language
/// setting lives in the archive, which is not open yet.
pub(crate) fn missing_runtime_notice() -> String {
    format!(
        "EntropIA necesita Microsoft Edge WebView2 Runtime, que no está instalado en este equipo.\n\
         ¿Desea abrir la descarga oficial de Microsoft? Después de instalarlo, vuelva a abrir EntropIA.\n\n\
         EntropIA needs the Microsoft Edge WebView2 Runtime, which is not installed on this computer.\n\
         Open Microsoft's official download? After installing it, open EntropIA again.\n\n\
         {DOWNLOAD_URL}"
    )
}

/// The installed WebView2 Runtime version, or `None` when there is none.
pub(crate) fn installed_version() -> Option<String> {
    let mut version = PWSTR::null();
    // SAFETY: the loader writes a CoTaskMem string into `version`, which
    // `take_pwstr` copies and frees.
    let found =
        unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut version) };
    if found.is_err() || version.is_null() {
        return None;
    }
    let version = webview2_com::take_pwstr(version);
    (!version.is_empty()).then_some(version)
}

/// Exit with a notice when the WebView2 Runtime is missing.
pub(crate) fn ensure_runtime() {
    if let Some(version) = installed_version() {
        log::info!(target: "webview2", "WebView2 Runtime {version}");
        return;
    }
    log::error!(target: "webview2", "WebView2 Runtime is not installed");
    // SAFETY: plain Win32 calls with strings that outlive them.
    unsafe {
        let answer = MessageBoxW(
            None,
            &HSTRING::from(missing_runtime_notice()),
            &HSTRING::from(TITLE),
            MB_YESNO | MB_ICONERROR | MB_SETFOREGROUND,
        );
        if answer == IDYES {
            ShellExecuteW(
                None,
                &HSTRING::from("open"),
                &HSTRING::from(DOWNLOAD_URL),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
    }
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notice_names_the_runtime_and_the_download_in_both_languages() {
        let notice = missing_runtime_notice();
        assert!(notice.contains("no está instalado"));
        assert!(notice.contains("is not installed"));
        assert!(notice.contains("Microsoft Edge WebView2 Runtime"));
        assert!(notice.ends_with(DOWNLOAD_URL));
        assert!(DOWNLOAD_URL.starts_with("https://go.microsoft.com/"));
    }

    #[test]
    fn a_machine_with_webview2_reports_its_version() {
        // Every supported Windows build and every CI runner has the runtime.
        let version = installed_version().expect("WebView2 Runtime version");
        assert!(version.split('.').count() >= 3, "{version}");
    }
}
