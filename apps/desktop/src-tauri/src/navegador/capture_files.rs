//! Where saved captures live on disk, and the only way to reach into it.
//!
//! Layout: `<data>/web-captures/<source_id>/<files>`. Both the delete command
//! and the startup sweep act on that tree, so the rules that keep them inside
//! it live here once: an id is a short run of `[A-Za-z0-9-]` (never a path),
//! a folder that is a symlink or junction is never entered, and the folder's
//! canonical location must be inside the canonical `web-captures` root.

use std::fs;
use std::path::{Path, PathBuf};

/// Longest source id accepted. Ids are UUIDs (36 characters).
pub const ID_MAX_LEN: usize = 64;

/// Whether `id` can name a source folder: short, and only `[A-Za-z0-9-]`, so it
/// can never climb out of the root or name a drive, a stream or a device.
pub fn valid_source_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= ID_MAX_LEN
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The folder every saved capture lives under.
pub fn root(data_dir: &Path) -> PathBuf {
    data_dir.join(super::save::DIR)
}

/// What looking for a source's folder found.
#[derive(Debug, PartialEq, Eq)]
pub enum Located {
    /// There is no folder: nothing to do.
    Missing,
    /// A real folder inside the root.
    Dir(PathBuf),
    /// Something is there that must not be touched, and why.
    Refused(&'static str),
}

/// Find the folder of `source_id` under `<data_dir>/web-captures/`, refusing
/// anything that is not a plain folder inside it.
pub fn locate_source_dir(data_dir: &Path, source_id: &str) -> Located {
    if !valid_source_id(source_id) {
        return Located::Refused("the source id is not valid");
    }
    let root = root(data_dir);
    match fs::symlink_metadata(&root) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Located::Refused("the captures folder is a link")
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Located::Missing,
        Err(_) => return Located::Refused("the captures folder cannot be inspected"),
    }
    let dir = root.join(source_id);
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Located::Refused("the source folder is a link")
        }
        Ok(meta) if !meta.is_dir() => return Located::Refused("the source folder is not a folder"),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Located::Missing,
        Err(_) => return Located::Refused("the source folder cannot be inspected"),
    }
    // Belt and braces for reparse points the metadata did not call links: where
    // the folder really is must be inside where the root really is.
    match (fs::canonicalize(&root), fs::canonicalize(&dir)) {
        (Ok(real_root), Ok(real_dir)) if real_dir.starts_with(&real_root) => Located::Dir(dir),
        _ => Located::Refused("the source folder resolves outside the captures folder"),
    }
}

/// What removing a folder's contents did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Removal {
    /// Regular files deleted.
    pub files: usize,
    /// Entries left in place (links, folders, files that would not go).
    pub left: usize,
    /// The folder itself is gone.
    pub dir_removed: bool,
}

/// Delete the regular files directly inside `dir` and then the folder, if it is
/// empty. Links and sub-folders are never followed or removed: they count as
/// `left`. `dir` must already come from [`locate_source_dir`].
pub fn remove_dir_contents(dir: &Path) -> Removal {
    let mut removal = Removal::default();
    let Ok(entries) = fs::read_dir(dir) else {
        removal.left += 1;
        return removal;
    };
    for entry in entries.flatten() {
        let is_file = entry.file_type().is_ok_and(|kind| kind.is_file());
        if is_file && fs::remove_file(entry.path()).is_ok() {
            removal.files += 1;
        } else {
            removal.left += 1;
        }
    }
    if removal.left == 0 {
        removal.dir_removed = fs::remove_dir(dir).is_ok();
    }
    removal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_short_plain_ids_name_a_folder() {
        assert!(valid_source_id("3f2a9c1e-7b1d-4c55-8d0a-0123456789ab"));
        assert!(valid_source_id("abc123"));
        for bad in [
            "",
            ".",
            "..",
            "a/b",
            "a\\b",
            "a b",
            "a:b",
            "a.b",
            "C:",
            "con.txt",
            "ñ",
            &"a".repeat(ID_MAX_LEN + 1),
        ] {
            assert!(!valid_source_id(bad), "{bad:?} must be refused");
        }
    }

    #[test]
    fn a_missing_folder_is_not_an_error() {
        let data = tempfile::tempdir().unwrap();
        assert_eq!(locate_source_dir(data.path(), "src-1"), Located::Missing);
        fs::create_dir_all(root(data.path())).unwrap();
        assert_eq!(locate_source_dir(data.path(), "src-1"), Located::Missing);
    }

    #[test]
    fn a_plain_folder_is_found_and_a_bad_id_is_refused() {
        let data = tempfile::tempdir().unwrap();
        let dir = root(data.path()).join("src-1");
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(locate_source_dir(data.path(), "src-1"), Located::Dir(dir));
        assert!(matches!(
            locate_source_dir(data.path(), "../src-1"),
            Located::Refused(_)
        ));
    }

    #[test]
    fn a_file_where_the_folder_should_be_is_refused() {
        let data = tempfile::tempdir().unwrap();
        fs::create_dir_all(root(data.path())).unwrap();
        fs::write(root(data.path()).join("src-1"), b"x").unwrap();
        assert!(matches!(
            locate_source_dir(data.path(), "src-1"),
            Located::Refused(_)
        ));
    }

    #[test]
    fn removing_a_folder_deletes_its_files_and_the_folder() {
        let data = tempfile::tempdir().unwrap();
        let dir = root(data.path()).join("src-1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.html"), b"a").unwrap();
        fs::write(dir.join(".b.tmp"), b"b").unwrap();

        let removal = remove_dir_contents(&dir);

        assert_eq!(
            removal,
            Removal {
                files: 2,
                left: 0,
                dir_removed: true
            }
        );
        assert!(!dir.exists());
    }

    #[test]
    fn a_sub_folder_is_left_alone_and_keeps_the_folder() {
        let data = tempfile::tempdir().unwrap();
        let dir = root(data.path()).join("src-1");
        fs::create_dir_all(dir.join("inner")).unwrap();
        fs::write(dir.join("a.html"), b"a").unwrap();

        let removal = remove_dir_contents(&dir);

        assert_eq!(removal.files, 1);
        assert_eq!(removal.left, 1);
        assert!(!removal.dir_removed);
        assert!(dir.join("inner").is_dir());
    }
}
