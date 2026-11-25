//! Removal of the runtime folders the running app no longer uses.
//!
//! A runtime is several GB of small files, and on Windows any of them can be
//! held by a process (the Python of an old venv, an antivirus scan). Deleting
//! such a folder in place stops halfway: the files already gone include
//! `manifest.json`, so what is left can no longer be told apart from anything
//! else and stays on disk for good.
//!
//! Removal is therefore two-phase:
//!
//! 1. [`retire_unused_runtimes`] renames every unused folder to
//!    `.retired-<name>-<millis>`. Renaming is instant and all-or-nothing, so
//!    the folder keeps every file under one marked name, and this phase can
//!    run under the runtime operations lock.
//! 2. [`purge_retired_runtimes`] deletes the `.retired-*` folders. It needs no
//!    lock: nothing reads a folder whose name starts with `.` as a runtime. A
//!    folder that cannot be deleted completely keeps its marked name, and the
//!    next purge finishes it.
//!
//! Windows does not stop either phase on its own when a program runs from the
//! folder: a folder whose `python.exe` is running can be renamed, and
//! deleting it then removes every file but the running ones. So both phases
//! first list the executables in use (every running process's executable and
//! every module loaded into this process) and leave alone any folder that
//! holds one of them; a later sweep tries again. Nothing starts a program
//! from an unused runtime once the active one is healthy, which is when the
//! sweep runs.
//!
//! A folder without `manifest.json` is the leftover of a deletion that stopped
//! halfway (the code before this module deleted in place); it is retired too.
//!
//! Each download also leaves its archive (about 2.3 GB) under
//! `.downloads/<pack>`, which nothing reads once the runtime is active.
//! [`remove_finished_downloads`] deletes those, under the same lock as the
//! retire phase, and keeps a paused download so it can resume.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::runtime::manifest::RuntimeManifest;
use crate::runtime::paths::runtime_root;

/// Name prefix of a folder marked for deletion.
pub(crate) const RETIRED_PREFIX: &str = ".retired-";

/// What the retire phase did.
#[derive(Debug, Default)]
pub(crate) struct RetireReport {
    /// The original paths of the folders renamed for deletion.
    pub retired: Vec<PathBuf>,
    /// One message per unused folder that could not be renamed (kept intact).
    pub kept: Vec<String>,
}

/// What the purge phase did.
#[derive(Debug, Default)]
pub(crate) struct PurgeReport {
    /// The `.retired-*` folders deleted completely.
    pub removed: Vec<PathBuf>,
    /// One message per `.retired-*` folder still on disk.
    pub failed: Vec<String>,
}

/// Renames to `.retired-*` every folder under `<app_data_dir>/runtime` that
/// the running app does not use.
///
/// The caller must hold the runtime operations lock, so no download, hydration
/// or dependency install is writing under `runtime/` meanwhile.
///
/// Kept: every path in `keep` (the active runtime), `downloads` (resumable
/// archive parts), a runtime of another platform, a folder whose manifest
/// exists but cannot be read, any other `.`-prefixed folder (such as
/// `.downloads`), and any folder a running program uses. Of this platform's
/// runtimes, those built for `app_version` are kept; while there is none, so
/// are those `serves_running_app` accepts (a runtime reused from another app
/// version, see `runtime_serves_running_app` in the manager).
/// Retired: every other runtime of this platform, a folder without
/// `manifest.json`, and the `.<pack>.staging` / `.<pack>.replacing-*`
/// folders an interrupted activation left behind.
pub(crate) fn retire_unused_runtimes(
    app_data_dir: &Path,
    keep: &[PathBuf],
    platform: &str,
    app_version: &str,
    serves_running_app: &dyn Fn(&RuntimeManifest) -> bool,
) -> RetireReport {
    let Some(images) = images_in_use() else {
        return RetireReport {
            retired: Vec::new(),
            kept: vec![PROCESS_LIST_UNAVAILABLE.to_string()],
        };
    };
    retire_unused_runtimes_with(
        app_data_dir,
        keep,
        &RunningApp {
            platform,
            app_version,
            serves_running_app,
        },
        &images,
        |from, to| std::fs::rename(from, to),
    )
}

/// Retires the one runtime folder the user replaced (the CPU runtime once
/// the GPU one is healthy). The regular sweep keeps it: it still serves the
/// running app. A folder a running program uses stays, as in the sweep.
pub(crate) fn retire_replaced_runtime(app_data_dir: &Path, folder: &Path) -> RetireReport {
    let Some(images) = images_in_use() else {
        return RetireReport {
            retired: Vec::new(),
            kept: vec![PROCESS_LIST_UNAVAILABLE.to_string()],
        };
    };
    retire_replaced_runtime_with(app_data_dir, folder, &images, |from, to| {
        std::fs::rename(from, to)
    })
}

fn retire_replaced_runtime_with<R>(
    app_data_dir: &Path,
    folder: &Path,
    images: &[PathBuf],
    mut rename: R,
) -> RetireReport
where
    R: FnMut(&Path, &Path) -> io::Result<()>,
{
    let mut report = RetireReport::default();
    let root = runtime_root(app_data_dir);
    let Some(name) = folder.file_name().and_then(|name| name.to_str()) else {
        return report;
    };
    if folder.parent() != Some(root.as_path()) || name.starts_with('.') || !folder.is_dir() {
        return report;
    }
    if holds_image(folder, images) {
        report.kept.push(format!(
            "Runtime folder {} is used by a running program and stays for a later cleanup",
            folder.display()
        ));
        return report;
    }
    match rename(folder, &retired_path(&root, name)) {
        Ok(()) => report.retired.push(folder.to_path_buf()),
        Err(error) => report.kept.push(format!(
            "Runtime folder {} is in use and stays for a later cleanup: {error}",
            folder.display()
        )),
    }
    report
}

/// What decides whether a runtime folder is still of use.
struct RunningApp<'a> {
    platform: &'a str,
    app_version: &'a str,
    serves_running_app: &'a dyn Fn(&RuntimeManifest) -> bool,
}

/// Why a sweep did nothing when the running programs could not be listed.
const PROCESS_LIST_UNAVAILABLE: &str =
    "Could not list the running programs; no runtime folder is touched this time";

fn retire_unused_runtimes_with<R>(
    app_data_dir: &Path,
    keep: &[PathBuf],
    app: &RunningApp<'_>,
    images: &[PathBuf],
    mut rename: R,
) -> RetireReport
where
    R: FnMut(&Path, &Path) -> io::Result<()>,
{
    let mut report = RetireReport::default();
    let root = runtime_root(app_data_dir);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return report;
    };
    let folders: Vec<(PathBuf, String)> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            Some((entry.path(), name))
        })
        .collect();
    // A runtime reused from another app version is of use only while no
    // runtime for this exact version exists.
    let exact_version_present = folders.iter().any(|(path, name)| {
        !name.starts_with('.')
            && RuntimeManifest::load_from_path(&path.join("manifest.json")).is_ok_and(|manifest| {
                manifest.platform == app.platform && manifest.app_version == app.app_version
            })
    });
    let mut candidates: Vec<(PathBuf, String)> = folders
        .into_iter()
        .filter(|(path, name)| is_unused(path, name, keep, app, exact_version_present))
        .collect();
    candidates.sort();

    for (path, name) in candidates {
        if holds_image(&path, images) {
            report.kept.push(format!(
                "Runtime folder {} is used by a running program and stays for a later cleanup",
                path.display()
            ));
            continue;
        }
        let target = retired_path(&root, &name);
        match rename(&path, &target) {
            Ok(()) => report.retired.push(path),
            Err(error) => report.kept.push(format!(
                "Runtime folder {} is in use and stays for a later cleanup: {error}",
                path.display()
            )),
        }
    }
    report
}

fn is_unused(
    path: &Path,
    name: &str,
    keep: &[PathBuf],
    app: &RunningApp<'_>,
    exact_version_present: bool,
) -> bool {
    if name.starts_with(RETIRED_PREFIX) {
        // Already marked: the purge phase deletes it.
        return false;
    }
    if let Some(rest) = name.strip_prefix('.') {
        return rest.ends_with(".staging") || rest.contains(".replacing-");
    }
    if name == "downloads" || keep.iter().any(|kept| kept == path) {
        return false;
    }
    let manifest_path = path.join("manifest.json");
    if !manifest_path.exists() {
        return true;
    }
    match RuntimeManifest::load_from_path(&manifest_path) {
        Ok(manifest) => {
            let of_use = if exact_version_present {
                manifest.app_version == app.app_version
            } else {
                (app.serves_running_app)(&manifest)
            };
            manifest.platform == app.platform && !of_use
        }
        // A manifest that exists but cannot be read now (an antivirus holding
        // it, a disk hiccup) proves nothing: keep the folder.
        Err(_) => false,
    }
}

fn retired_path(root: &Path, name: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let base = format!("{RETIRED_PREFIX}{name}-{millis}");
    let mut candidate = root.join(&base);
    let mut counter = 1u32;
    while candidate.exists() {
        candidate = root.join(format!("{base}-{counter}"));
        counter += 1;
    }
    candidate
}

/// Deletes every `.retired-*` folder under `<app_data_dir>/runtime` that no
/// running program uses.
pub(crate) fn purge_retired_runtimes(app_data_dir: &Path) -> PurgeReport {
    let Some(images) = images_in_use() else {
        return PurgeReport {
            removed: Vec::new(),
            failed: vec![PROCESS_LIST_UNAVAILABLE.to_string()],
        };
    };
    purge_retired_runtimes_with(
        app_data_dir,
        &images,
        crate::path_utils::remove_dir_all_robust,
    )
}

fn purge_retired_runtimes_with<D>(
    app_data_dir: &Path,
    images: &[PathBuf],
    mut remove_dir_all: D,
) -> PurgeReport
where
    D: FnMut(&Path) -> io::Result<()>,
{
    let mut report = PurgeReport::default();
    let Ok(entries) = std::fs::read_dir(runtime_root(app_data_dir)) else {
        return report;
    };
    let mut retired: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(RETIRED_PREFIX))
        })
        .map(|entry| entry.path())
        .collect();
    retired.sort();

    for path in retired {
        if holds_image(&path, images) {
            report.failed.push(format!(
                "{} is still used by a running program; deleted on a later cleanup",
                path.display()
            ));
            continue;
        }
        match remove_dir_all(&path) {
            Ok(()) => report.removed.push(path),
            Err(error) => report.failed.push(format!(
                "Could not finish deleting {} (retried on the next cleanup): {error}",
                path.display()
            )),
        }
    }
    report
}

/// Deletes every `.downloads/<pack>` folder that holds no download in
/// progress.
///
/// A finished download keeps its archive (about 2.3 GB) and `resume.json`,
/// and nothing reads either once the runtime is active: an interrupted
/// download resumes from its `*.download.tmp` file and
/// `resume-state.json`. So a folder without those two is dead weight, one per
/// runtime version ever installed. A folder with them is a paused download
/// and stays, so it can resume.
///
/// The caller must hold the runtime operations lock, so no download is
/// writing under `.downloads` meanwhile.
pub(crate) fn remove_finished_downloads(app_data_dir: &Path) -> PurgeReport {
    let mut report = PurgeReport::default();
    let Ok(entries) = std::fs::read_dir(runtime_root(app_data_dir).join(".downloads")) else {
        return report;
    };
    let mut finished: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .filter(|folder| !holds_a_download_in_progress(folder))
        .collect();
    finished.sort();
    for folder in finished {
        match crate::path_utils::remove_dir_all_robust(&folder) {
            Ok(()) => report.removed.push(folder),
            Err(error) => report.failed.push(format!(
                "Finished runtime download {} stays for a later cleanup: {error}",
                folder.display()
            )),
        }
    }
    report
}

/// Whether a `.downloads/<pack>` folder holds the staged bytes or the resume
/// state of a download that has not finished. A folder that cannot be listed
/// counts as one, so it is left alone.
fn holds_a_download_in_progress(folder: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return true;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name == crate::runtime::download_state::RESUME_STATE_FILE_NAME
            || name.ends_with(".download.tmp")
    })
}

/// Whether one of `images` (executable paths) lies inside `folder`.
/// Compared as Windows paths: case-insensitive, either slash, with or
/// without the `\\?\` prefix.
fn holds_image(folder: &Path, images: &[PathBuf]) -> bool {
    fn normalized(path: &Path) -> String {
        let text = path.to_string_lossy().replace('/', "\\");
        let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
        text.trim_end_matches('\\').to_lowercase()
    }
    let prefix = format!("{}\\", normalized(folder));
    images
        .iter()
        .any(|image| normalized(image).starts_with(&prefix))
}

/// The executables in use: the image of every running process this user can
/// query, and every module loaded into this process (the ONNX Runtime DLL,
/// for one). `None` when the process list cannot be read.
///
/// Only Windows needs it: elsewhere a running program keeps the files it has
/// open after they are deleted, so a sweep never breaks it.
#[cfg(windows)]
fn images_in_use() -> Option<Vec<PathBuf>> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW, Process32NextW,
        MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    fn wide_path(buffer: &[u16]) -> PathBuf {
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        PathBuf::from(OsString::from_wide(&buffer[..len]))
    }

    let mut images = Vec::new();
    let mut name = vec![0u16; 32_768];

    // SAFETY: every handle comes from the call above its use, is checked
    // against its failure value, and is closed exactly once; the entry
    // structs are zero-initialized plain data with `dwSize` set as the API
    // requires, and `name` outlives the call that fills it with its length.
    unsafe {
        let processes = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if processes == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut process: PROCESSENTRY32W = std::mem::zeroed();
        process.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(processes, &mut process) != 0;
        while more {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process.th32ProcessID);
            if !handle.is_null() {
                let mut len = name.len() as u32;
                if QueryFullProcessImageNameW(
                    handle,
                    PROCESS_NAME_WIN32,
                    name.as_mut_ptr(),
                    &mut len,
                ) != 0
                {
                    images.push(wide_path(&name[..len as usize]));
                }
                CloseHandle(handle);
            }
            more = Process32NextW(processes, &mut process) != 0;
        }
        CloseHandle(processes);

        // A module snapshot can fail transiently while this process loads a
        // library (ERROR_BAD_LENGTH): retry a few times before giving up.
        let mut modules = INVALID_HANDLE_VALUE;
        for _ in 0..5 {
            modules = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE, 0);
            if modules != INVALID_HANDLE_VALUE {
                break;
            }
        }
        if modules == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut module: MODULEENTRY32W = std::mem::zeroed();
        module.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;
        let mut more = Module32FirstW(modules, &mut module) != 0;
        while more {
            images.push(wide_path(&module.szExePath));
            more = Module32NextW(modules, &mut module) != 0;
        }
        CloseHandle(modules);
    }
    Some(images)
}

#[cfg(not(windows))]
fn images_in_use() -> Option<Vec<PathBuf>> {
    Some(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    const PLATFORM: &str = "windows-x86_64";
    const RUNNING: &str = "1.8.6";

    /// The plain rule: only a runtime built for the running version serves it.
    fn exact_version(manifest: &RuntimeManifest) -> bool {
        manifest.app_version == RUNNING
    }

    fn app() -> RunningApp<'static> {
        RunningApp {
            platform: PLATFORM,
            app_version: RUNNING,
            serves_running_app: &exact_version,
        }
    }

    fn retire(app_data_dir: &Path, keep: &[PathBuf]) -> RetireReport {
        retire_unused_runtimes(app_data_dir, keep, PLATFORM, RUNNING, &exact_version)
    }

    fn runtime(app_data_dir: &Path, name: &str) -> PathBuf {
        app_data_dir.join("runtime").join(name)
    }

    /// A runtime folder with a manifest and some content.
    fn write_runtime(
        app_data_dir: &Path,
        name: &str,
        platform: &str,
        app_version: &str,
    ) -> PathBuf {
        let root = runtime(app_data_dir, name);
        fs::create_dir_all(root.join("venv").join("Scripts")).unwrap();
        fs::write(root.join("venv").join("Scripts").join("python.exe"), b"py").unwrap();
        fs::write(
            root.join("manifest.json"),
            format!(
                r#"{{"pack_version":"{name}","app_version":"{app_version}","platform":"{platform}","python_relpath":"python/python.exe","uv_relpath":"uv/uv.exe"}}"#
            ),
        )
        .unwrap();
        root
    }

    /// A folder with content but no manifest: what an interrupted in-place
    /// deletion leaves behind.
    fn write_orphan(app_data_dir: &Path, name: &str) -> PathBuf {
        let root = runtime(app_data_dir, name);
        fs::create_dir_all(root.join("venv").join("entropia-env")).unwrap();
        fs::write(root.join("venv").join("entropia-env").join("x.pyd"), b"x").unwrap();
        root
    }

    fn names_under_runtime(app_data_dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(app_data_dir.join("runtime"))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// A finished download, as the bootstrap leaves it.
    fn write_finished_download(app_data_dir: &Path, pack: &str) -> PathBuf {
        let folder = runtime(app_data_dir, ".downloads").join(pack);
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join(format!("runtime-pack-windows-x86_64-{pack}.zip.part000")),
            b"zip",
        )
        .unwrap();
        fs::write(folder.join("resume.json"), b"{}").unwrap();
        folder
    }

    #[test]
    fn finished_downloads_are_deleted() {
        let dir = tempdir().unwrap();
        let old = write_finished_download(dir.path(), "2026.11.8");
        let active = write_finished_download(dir.path(), "2026.11.10");

        let report = remove_finished_downloads(dir.path());

        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert_eq!(report.removed, vec![active.clone(), old.clone()]);
        assert!(!old.exists());
        assert!(!active.exists());
    }

    #[test]
    fn a_paused_download_stays_so_it_can_resume() {
        let dir = tempdir().unwrap();
        let paused = runtime(dir.path(), ".downloads").join("2026.11.11");
        fs::create_dir_all(&paused).unwrap();
        fs::write(
            paused.join("runtime-pack-windows-x86_64-2026.11.11.zip.download.tmp"),
            b"part",
        )
        .unwrap();
        fs::write(
            paused.join(crate::runtime::download_state::RESUME_STATE_FILE_NAME),
            b"{}",
        )
        .unwrap();
        // Only the staged bytes, before the first state write.
        let starting = runtime(dir.path(), ".downloads").join("2026.11.12");
        fs::create_dir_all(&starting).unwrap();
        fs::write(
            starting.join("runtime-pack-windows-x86_64-2026.11.12.zip.download.tmp"),
            b"p",
        )
        .unwrap();
        let finished = write_finished_download(dir.path(), "2026.11.9");

        let report = remove_finished_downloads(dir.path());

        assert_eq!(report.removed, vec![finished]);
        assert!(paused
            .join("runtime-pack-windows-x86_64-2026.11.11.zip.download.tmp")
            .exists());
        assert!(starting.exists());
    }

    #[test]
    fn removing_finished_downloads_leaves_the_runtimes_alone() {
        let dir = tempdir().unwrap();
        let current = write_runtime(dir.path(), "2026.11.10", PLATFORM, RUNNING);
        let old = write_runtime(dir.path(), "2026.11.9", PLATFORM, "1.8.5");
        write_finished_download(dir.path(), "2026.11.10");

        remove_finished_downloads(dir.path());

        assert!(current.join("manifest.json").exists());
        assert!(old.join("manifest.json").exists());
        assert!(runtime(dir.path(), ".downloads").exists());
    }

    #[test]
    fn no_downloads_folder_is_nothing_to_remove() {
        let dir = tempdir().unwrap();
        let report = remove_finished_downloads(dir.path());
        assert!(report.removed.is_empty());
        assert!(report.failed.is_empty());
    }

    #[test]
    fn retires_other_app_versions_and_keeps_the_running_one() {
        let dir = tempdir().unwrap();
        let old = write_runtime(dir.path(), "2026.11.8", PLATFORM, "1.8.4");
        let current = write_runtime(dir.path(), "2026.11.10", PLATFORM, RUNNING);

        let report = retire(dir.path(), std::slice::from_ref(&current));

        assert_eq!(report.retired, vec![old.clone()]);
        assert!(report.kept.is_empty(), "{:?}", report.kept);
        assert!(!old.exists());
        assert!(current.join("manifest.json").exists());
        let names = names_under_runtime(dir.path());
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(
            names
                .iter()
                .any(|name| name.starts_with(".retired-2026.11.8-")),
            "{names:?}"
        );
    }

    #[test]
    fn a_folder_without_manifest_is_retired() {
        // The 1.8.5 field case: 2026.11.8 lost its manifest to a deletion that
        // stopped on a file in use, and was never cleaned again.
        let dir = tempdir().unwrap();
        let orphan = write_orphan(dir.path(), "2026.11.8");
        let current = write_runtime(dir.path(), "2026.11.9", PLATFORM, RUNNING);

        let report = retire(dir.path(), std::slice::from_ref(&current));

        assert_eq!(report.retired, vec![orphan.clone()]);
        assert!(!orphan.exists());
        assert!(current.exists());
    }

    #[test]
    fn keeps_the_listed_folders_downloads_other_platforms_and_unreadable_manifests() {
        let dir = tempdir().unwrap();
        // Listed in `keep` even though it has no manifest (the bundled pack
        // folder while it is being checked).
        let kept_without_manifest = write_orphan(dir.path(), "2026.11.10");
        let downloads = runtime(dir.path(), "downloads");
        fs::create_dir_all(&downloads).unwrap();
        fs::write(downloads.join("part-1.zip"), b"p").unwrap();
        let other_platform = write_runtime(dir.path(), "2026.11.7", "linux-x86_64", "1.8.4");
        let unreadable = runtime(dir.path(), "2026.11.6");
        fs::create_dir_all(&unreadable).unwrap();
        fs::write(unreadable.join("manifest.json"), b"{ not json").unwrap();
        let unknown_dot = runtime(dir.path(), ".cache");
        fs::create_dir_all(&unknown_dot).unwrap();
        // Where resumable downloads really live.
        let dot_downloads = runtime(dir.path(), ".downloads");
        fs::create_dir_all(dot_downloads.join("2026.11.10")).unwrap();
        fs::write(runtime(dir.path(), ".2026.11.10.stage.json"), b"{}").unwrap();

        let report = retire(dir.path(), std::slice::from_ref(&kept_without_manifest));

        assert!(report.retired.is_empty(), "{:?}", report.retired);
        for path in [
            &kept_without_manifest,
            &downloads,
            &other_platform,
            &unreadable,
            &unknown_dot,
            &dot_downloads,
        ] {
            assert!(path.exists(), "{} must stay", path.display());
        }
        assert!(runtime(dir.path(), ".2026.11.10.stage.json").exists());
    }

    #[test]
    fn retires_leftover_staging_and_replacement_folders() {
        let dir = tempdir().unwrap();
        let staging = runtime(dir.path(), ".2026.11.9.staging");
        let replacing = runtime(dir.path(), ".2026.11.9.replacing-1795190000000");
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir_all(&replacing).unwrap();

        let report = retire(dir.path(), &[]);

        assert_eq!(report.retired, vec![replacing.clone(), staging.clone()]);
        assert!(!staging.exists() && !replacing.exists());
    }

    #[test]
    fn a_folder_that_cannot_be_renamed_stays_complete() {
        let dir = tempdir().unwrap();
        let in_use = write_runtime(dir.path(), "2026.11.8", PLATFORM, "1.8.4");
        let free = write_runtime(dir.path(), "2026.11.7", PLATFORM, "1.8.3");

        let report = retire_unused_runtimes_with(dir.path(), &[], &app(), &[], |from, to| {
            if from == in_use {
                Err(io::Error::from_raw_os_error(5))
            } else {
                fs::rename(from, to)
            }
        });

        assert_eq!(report.retired, vec![free.clone()]);
        assert_eq!(report.kept.len(), 1, "{:?}", report.kept);
        assert!(report.kept[0].contains("2026.11.8"), "{:?}", report.kept);
        assert!(
            in_use.join("manifest.json").exists(),
            "a runtime in use is not touched at all"
        );
        assert!(in_use
            .join("venv")
            .join("Scripts")
            .join("python.exe")
            .exists());
    }

    #[test]
    fn already_retired_folders_are_left_to_the_purge() {
        let dir = tempdir().unwrap();
        let retired = runtime(dir.path(), ".retired-2026.11.5-1");
        fs::create_dir_all(&retired).unwrap();

        let report = retire(dir.path(), &[]);

        assert!(report.retired.is_empty());
        assert!(retired.exists());
    }

    #[test]
    fn missing_runtime_folder_is_a_no_op() {
        let dir = tempdir().unwrap();

        let retire = retire(dir.path(), &[]);
        let purge = purge_retired_runtimes(dir.path());

        assert!(retire.retired.is_empty() && retire.kept.is_empty());
        assert!(purge.removed.is_empty() && purge.failed.is_empty());
    }

    #[test]
    fn purge_deletes_only_retired_folders() {
        let dir = tempdir().unwrap();
        let current = write_runtime(dir.path(), "2026.11.10", PLATFORM, RUNNING);
        let retired = runtime(dir.path(), ".retired-2026.11.8-1795190000000");
        fs::create_dir_all(retired.join("venv")).unwrap();
        fs::write(retired.join("venv").join("python.exe"), b"py").unwrap();
        let staging = runtime(dir.path(), ".2026.11.10.staging");
        fs::create_dir_all(&staging).unwrap();

        let report = purge_retired_runtimes(dir.path());

        assert_eq!(report.removed, vec![retired.clone()]);
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert!(!retired.exists());
        assert!(current.join("manifest.json").exists());
        assert!(staging.exists(), "the purge only deletes marked folders");
    }

    #[test]
    fn a_retired_folder_that_resists_deletion_is_retried_by_the_next_purge() {
        let dir = tempdir().unwrap();
        let retired = runtime(dir.path(), ".retired-2026.11.8-1");
        fs::create_dir_all(&retired).unwrap();
        fs::write(retired.join("held.dll"), b"x").unwrap();

        let first =
            purge_retired_runtimes_with(dir.path(), &[], |_| Err(io::Error::from_raw_os_error(5)));
        assert_eq!(first.failed.len(), 1, "{:?}", first.failed);
        assert!(retired.exists(), "it keeps its marked name");

        let second = purge_retired_runtimes(dir.path());
        assert_eq!(second.removed, vec![retired.clone()]);
        assert!(!retired.exists());
    }

    #[test]
    fn retire_then_purge_leaves_only_the_running_runtime() {
        let dir = tempdir().unwrap();
        write_orphan(dir.path(), "2026.11.8");
        write_runtime(dir.path(), "2026.11.5", PLATFORM, "1.8.2");
        let current = write_runtime(dir.path(), "2026.11.10", PLATFORM, RUNNING);

        retire(dir.path(), std::slice::from_ref(&current));
        purge_retired_runtimes(dir.path());

        assert_eq!(
            names_under_runtime(dir.path()),
            vec!["2026.11.10".to_string()]
        );
    }

    /// A runtime built for 1.8.4 that this version can reuse (same ABI and
    /// scripts, decided by the manager): kept while no 1.8.6 runtime exists.
    #[test]
    fn a_reusable_runtime_of_another_version_is_kept_while_none_is_exact() {
        let dir = tempdir().unwrap();
        let reusable = write_runtime(dir.path(), "2026.11.9", PLATFORM, "1.8.4");
        let not_reusable = write_runtime(dir.path(), "2026.11.8", PLATFORM, "1.8.3");
        let serves = |manifest: &RuntimeManifest| {
            manifest.app_version == RUNNING || manifest.app_version == "1.8.4"
        };

        let report = retire_unused_runtimes(dir.path(), &[], PLATFORM, RUNNING, &serves);

        assert_eq!(report.retired, vec![not_reusable]);
        assert!(reusable.join("manifest.json").exists());
    }

    /// Once a runtime for this exact version exists (the reused one was
    /// damaged and a new one was downloaded), the reused one is dead weight.
    #[test]
    fn a_reused_runtime_goes_once_one_for_this_version_exists() {
        let dir = tempdir().unwrap();
        let reused = write_runtime(dir.path(), "2026.11.9", PLATFORM, "1.8.4");
        let exact = write_runtime(dir.path(), "2026.11.10", PLATFORM, RUNNING);
        let serves = |manifest: &RuntimeManifest| {
            manifest.app_version == RUNNING || manifest.app_version == "1.8.4"
        };

        let report = retire_unused_runtimes(
            dir.path(),
            std::slice::from_ref(&exact),
            PLATFORM,
            RUNNING,
            &serves,
        );

        assert_eq!(report.retired, vec![reused]);
        assert!(exact.join("manifest.json").exists());
    }

    #[test]
    fn a_folder_a_program_runs_from_is_not_retired() {
        let dir = tempdir().unwrap();
        let in_use = write_runtime(dir.path(), "2026.11.8", PLATFORM, "1.8.4");
        let free = write_runtime(dir.path(), "2026.11.7", PLATFORM, "1.8.3");
        // Spelled the way Windows reports a process image.
        let images = [PathBuf::from(
            in_use
                .join("venv")
                .join("Scripts")
                .join("python.exe")
                .to_string_lossy()
                .to_uppercase(),
        )];

        let report = retire_unused_runtimes_with(dir.path(), &[], &app(), &images, |from, to| {
            fs::rename(from, to)
        });

        assert_eq!(report.retired, vec![free]);
        assert_eq!(report.kept.len(), 1, "{:?}", report.kept);
        assert!(in_use.join("manifest.json").exists());
    }

    #[test]
    fn a_retired_folder_a_program_runs_from_is_not_deleted() {
        let dir = tempdir().unwrap();
        let retired = runtime(dir.path(), ".retired-2026.11.8-1");
        fs::create_dir_all(retired.join("python")).unwrap();
        fs::write(retired.join("python").join("python.exe"), b"py").unwrap();
        fs::write(retired.join("python").join("python3.dll"), b"dll").unwrap();
        let images = [retired.join("python").join("python.exe")];

        let report = purge_retired_runtimes_with(dir.path(), &images, |path| {
            panic!("{} must not be deleted while in use", path.display())
        });

        assert!(report.removed.is_empty());
        assert_eq!(report.failed.len(), 1, "{:?}", report.failed);
        assert!(retired.join("python").join("python3.dll").exists());
    }

    #[test]
    fn an_image_only_counts_inside_the_folder_itself() {
        let folder = Path::new(r"C:\Data\runtime\2026.11.8");
        let held = |image: &str| holds_image(folder, &[PathBuf::from(image)]);

        assert!(held(r"C:\Data\runtime\2026.11.8\python\python.exe"));
        assert!(held(r"\\?\c:\data\RUNTIME\2026.11.8\uv\uv.exe"));
        assert!(held("C:/Data/runtime/2026.11.8/venv/Scripts/python.exe"));
        // A sibling whose name starts the same is another folder.
        assert!(!held(r"C:\Data\runtime\2026.11.80\python\python.exe"));
        assert!(!held(r"C:\Data\runtime\2026.11.9\python\python.exe"));
        assert!(!held(r"C:\Data\runtime\2026.11.8"));
    }

    /// The process list really sees this process: the test executable and
    /// its folder count as in use.
    #[cfg(windows)]
    #[test]
    fn the_process_list_includes_this_process() {
        let images = images_in_use().expect("the process list is readable");
        let exe = std::env::current_exe().unwrap();

        assert!(holds_image(exe.parent().unwrap(), &images), "{images:?}");
    }

    /// The case that made the check necessary, against the real OS: Windows
    /// lets a folder be renamed while a program runs from it, so the sweep
    /// itself must notice the running program and leave the folder whole.
    #[cfg(windows)]
    #[test]
    fn a_runtime_whose_program_is_running_stays_whole_until_it_exits() {
        let dir = tempdir().unwrap();
        let in_use = write_runtime(dir.path(), "2026.11.8", PLATFORM, "1.8.4");
        let system_root = std::env::var_os("SystemRoot").expect("SystemRoot is set");
        let program = in_use.join("venv").join("Scripts").join("waiter.exe");
        fs::copy(
            Path::new(&system_root).join("System32").join("PING.EXE"),
            &program,
        )
        .unwrap();
        let mut child = std::process::Command::new(&program)
            .args(["-n", "60", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();

        let report = retire(dir.path(), &[]);
        let still_whole = in_use.join("manifest.json").exists() && program.exists();
        child.kill().unwrap();
        child.wait().unwrap();

        assert!(report.retired.is_empty(), "{:?}", report.retired);
        assert_eq!(report.kept.len(), 1, "{:?}", report.kept);
        assert!(still_whole);

        let report = retire(dir.path(), &[]);
        assert_eq!(report.retired, vec![in_use]);
        let purge = purge_retired_runtimes(dir.path());
        assert!(purge.failed.is_empty(), "{:?}", purge.failed);
        assert!(names_under_runtime(dir.path()).is_empty());
    }

    #[test]
    fn a_replaced_runtime_is_retired_unless_a_running_program_uses_it() {
        let dir = tempdir().unwrap();
        let replaced = write_runtime(dir.path(), "2026.11.10", PLATFORM, RUNNING);
        let in_use = vec![replaced.join("venv").join("Scripts").join("python.exe")];

        let kept = retire_replaced_runtime_with(dir.path(), &replaced, &in_use, |from, to| {
            fs::rename(from, to)
        });
        assert!(kept.retired.is_empty());
        assert_eq!(kept.kept.len(), 1, "{:?}", kept.kept);
        assert!(replaced.join("manifest.json").is_file());

        let retired = retire_replaced_runtime_with(dir.path(), &replaced, &[], |from, to| {
            fs::rename(from, to)
        });
        assert_eq!(retired.retired, vec![replaced.clone()]);
        assert!(!replaced.exists());
        assert!(names_under_runtime(dir.path())
            .iter()
            .any(|name| name.starts_with(RETIRED_PREFIX)));
    }
}
