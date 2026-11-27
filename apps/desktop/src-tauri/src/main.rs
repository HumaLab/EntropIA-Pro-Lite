// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // On Windows, suppress CRT assertion dialogs and crash popups during
    // native library initialization. Libraries like llama.cpp, pdfium, and
    // stb_image use mmap and low-level file I/O that can trigger
    // _osfile(fh) & FOPEN assertions in MSVC Debug builds. These are
    // benign — the file handles are valid at the OS level, but the CRT's
    // debug tracking gets confused. Setting the error mode prevents modal
    // dialogs from blocking the app. Errors are still logged to stderr.
    #[cfg(target_os = "windows")]
    unsafe {
        // SetErrorMode constants
        const SEM_FAILCRITICALERRORS: u32 = 0x0001;
        const SEM_NOGPFAULTERRORBOX: u32 = 0x0002;
        const SEM_NOOPENFILEERRORBOX: u32 = 0x8000;

        extern "system" {
            fn SetErrorMode(uMode: u32) -> u32;
        }

        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX);
    }

    // `run()` routes MSVC debug-CRT assertions to stderr before any native
    // library loads.
    entropia_desktop_lib::run()
}
