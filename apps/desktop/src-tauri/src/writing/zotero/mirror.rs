//! A copy of the Zotero library kept on disk (plan-editor.md §11.2).
//!
//! Reading a library of thousands of works through the local API takes
//! seconds, and it used to happen every time the panel opened. This keeps the
//! last copy read, so the panel can list the library at once — even with Zotero
//! closed — and only asks Zotero what changed since.
//!
//! # What "changed" means here
//!
//! Every work carries a version, and the library carries the highest of them
//! (`Last-Modified-Version`). A library whose version has not moved has not
//! changed, which is one cheap request. When it has moved, the whole map of
//! `key → version` is one more (a few hundred KB even for a large library), and
//! comparing it with the copy says exactly which works to fetch and which to
//! drop — including works moved to the trash, which leave `/items/top` without
//! ever appearing in `/deleted`.
//!
//! A copy of a *different* library can only differ from the map, so it is
//! corrected by the same comparison rather than by trusting an identity
//! Zotero does not offer (S5: there is no `Zotero-Server-ID`).
//!
//! # Why it may be thrown away
//!
//! It lives in the cache directory: nothing here is the writer's own data. A
//! citation keeps its own snapshot of the work (§11.2), so a lost or corrupt
//! copy costs one full read and nothing else.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One work as the copy holds it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MirrorItem {
    /// Zotero's item key: what the version map and `itemKey` speak.
    pub key: String,
    pub version: u64,
    /// The work as CSL-JSON text — the same text the panel cites from.
    pub csl: String,
}

/// The copy of one library.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Mirror {
    pub library: String,
    /// The library's `Last-Modified-Version` when the copy was last brought up
    /// to date. `None` until the first complete read.
    pub version: Option<u64>,
    pub items: Vec<MirrorItem>,
}

/// What bringing the copy up to date requires.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Works that are new, or whose version moved.
    pub fetch: Vec<String>,
    /// Works the library no longer lists.
    pub remove: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.fetch.is_empty() && self.remove.is_empty()
    }
}

/// Past this many works to fetch, reading the whole library page by page is
/// cheaper than naming them fifty at a time (measured: 2,805 works took ~27 s
/// by key and 3–13 s by page).
pub const FULL_READ_THRESHOLD: usize = 300;

impl Mirror {
    pub fn empty(library: &str) -> Self {
        Self {
            library: library.to_string(),
            version: None,
            items: Vec::new(),
        }
    }

    /// The copy on disk, or an empty one.
    ///
    /// Missing, unreadable and "belongs to another library" all mean the same
    /// thing — there is no copy to trust — so none of them is an error.
    pub fn load(path: &Path, library: &str) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Mirror>(&bytes).ok())
            .filter(|mirror| mirror.library == library)
            .unwrap_or_else(|| Self::empty(library))
    }

    /// Writes the copy beside itself and renames it into place, so a crash
    /// mid-write leaves the previous copy rather than half of a new one.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let partial = path.with_extension("json.partial");
        std::fs::write(&partial, serde_json::to_vec(self)?)?;
        std::fs::rename(&partial, path)
    }

    /// Compares the copy with the library's map of `key → version`.
    pub fn plan(&self, remote: &HashMap<String, u64>) -> Plan {
        let held: HashMap<&str, u64> = self
            .items
            .iter()
            .map(|item| (item.key.as_str(), item.version))
            .collect();
        let fetch = remote
            .iter()
            .filter(|(key, version)| held.get(key.as_str()) != Some(*version))
            .map(|(key, _)| key.clone())
            .collect();
        let remove = self
            .items
            .iter()
            .filter(|item| !remote.contains_key(&item.key))
            .map(|item| item.key.clone())
            .collect();
        Plan { fetch, remove }
    }

    /// Replaces the fetched works, drops the removed ones, and records the
    /// library version the copy now matches.
    pub fn apply(&mut self, fetched: Vec<MirrorItem>, removed: &[String], version: Option<u64>) {
        let replaced: std::collections::HashSet<&str> = fetched
            .iter()
            .map(|item| item.key.as_str())
            .chain(removed.iter().map(String::as_str))
            .collect();
        self.items
            .retain(|item| !replaced.contains(item.key.as_str()));
        self.items.extend(fetched);
        self.version = version;
    }

    /// Replaces the whole copy with a complete read.
    pub fn replace(&mut self, items: Vec<MirrorItem>, version: Option<u64>) {
        self.items = items;
        self.version = version;
    }

    /// The works as CSL-JSON, most recently changed first — the same order
    /// Zotero lists a library in by default.
    pub fn csl(&self) -> Vec<String> {
        let mut ordered: Vec<&MirrorItem> = self.items.iter().collect();
        ordered.sort_by(|a, b| b.version.cmp(&a.version).then_with(|| a.key.cmp(&b.key)));
        ordered.into_iter().map(|item| item.csl.clone()).collect()
    }
}

/// The copy as the panel receives it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MirrorView {
    pub items: Vec<String>,
    pub version: Option<u64>,
}

/// What a sync did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SyncOutcome {
    /// The whole library, only when it changed: an unchanged library is not
    /// sent across again.
    pub items: Option<Vec<String>>,
    pub version: Option<u64>,
    pub fetched: usize,
    pub removed: usize,
}

impl Mirror {
    pub fn view(&self) -> MirrorView {
        MirrorView {
            items: self.csl(),
            version: self.version,
        }
    }
}

/// Whether bringing the copy up to date is cheaper as a complete read.
pub fn reads_whole_library(mirror: &Mirror, plan: &Plan) -> bool {
    mirror.items.is_empty() || plan.fetch.len() > FULL_READ_THRESHOLD
}

/// Brings the copy at `path` up to date with the library.
///
/// The library's version is read *before* anything else, and that is the
/// version recorded: a change made while this runs leaves the copy one
/// version behind, so the next sync picks it up instead of losing it.
pub async fn sync(
    client: &reqwest::Client,
    path: &Path,
    library: &str,
) -> Result<SyncOutcome, super::ZoteroState> {
    use super::connector;

    let (at, lib) = (path.to_path_buf(), library.to_string());
    let mut mirror = tokio::task::spawn_blocking(move || Mirror::load(&at, &lib))
        .await
        .unwrap_or_else(|_| Mirror::empty(library));

    let version = connector::library_version(client, library).await?;
    let unchanged = |version| SyncOutcome {
        items: None,
        version,
        fetched: 0,
        removed: 0,
    };
    if version.is_some() && version == mirror.version && !mirror.items.is_empty() {
        return Ok(unchanged(version));
    }

    let plan = mirror.plan(&connector::read_versions(client, library).await?);
    let (fetched, removed) = (plan.fetch.len(), plan.remove.len());
    if plan.is_empty() {
        // Something outside the works moved the version (a collection, a
        // saved search). The works are as they were.
        mirror.version = version;
        persist(mirror, path).await;
        return Ok(unchanged(version));
    }

    if reads_whole_library(&mirror, &plan) {
        let items = connector::read_library(client, library).await?;
        mirror.replace(items, version);
    } else {
        let items = connector::read_works(client, library, plan.fetch).await?;
        mirror.apply(items, &plan.remove, version);
    }

    let items = mirror.csl();
    persist(mirror, path).await;
    Ok(SyncOutcome {
        items: Some(items),
        version,
        fetched,
        removed,
    })
}

/// Saves the copy, and says so if it could not.
///
/// A copy that failed to save is not a failed sync: the panel already has the
/// library, and the next sync reads what the missing copy would have saved.
async fn persist(mirror: Mirror, path: &Path) {
    let path = path.to_path_buf();
    let saved = tokio::task::spawn_blocking(move || mirror.save(&path)).await;
    if let Ok(Err(error)) | Err(error) = saved.map_err(std::io::Error::other) {
        eprintln!("[zotero] the copy of the library could not be saved: {error}");
    }
}

/// Where the copy of a library lives inside the cache directory.
pub fn path_for(cache_dir: &Path, library: &str) -> PathBuf {
    // Only what can be part of a library id survives, so the id cannot name
    // a directory or climb out of this one.
    let safe: String = library
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    cache_dir
        .join("zotero")
        .join(format!("library-{safe}.json"))
}

/// Reads one item of a `format=json&include=csljson` answer.
///
/// Zotero sends the CSL as *text* holding a one-element array. It is parsed
/// and written back compactly, which is the same text a `format=csljson` read
/// produces, so a citation made from the copy is indistinguishable from one
/// made from a live read.
pub fn item_from_json(value: &serde_json::Value) -> Option<MirrorItem> {
    let key = value.get("key")?.as_str()?.to_string();
    let version = value.get("version")?.as_u64()?;
    let text = value.get("csljson")?.as_str()?;
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    let csl = match parsed {
        serde_json::Value::Array(mut items) if items.len() == 1 => items.remove(0),
        object @ serde_json::Value::Object(_) => object,
        _ => return None,
    };
    Some(MirrorItem {
        key,
        version,
        csl: csl.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, version: u64) -> MirrorItem {
        MirrorItem {
            key: key.into(),
            version,
            csl: format!(r#"{{"id":"{key}","title":"Work {key}"}}"#),
        }
    }

    fn mirror(items: &[(&str, u64)]) -> Mirror {
        let mut mirror = Mirror::empty("0");
        mirror.replace(items.iter().map(|(k, v)| item(k, *v)).collect(), Some(100));
        mirror
    }

    fn remote(items: &[(&str, u64)]) -> HashMap<String, u64> {
        items.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "entropia-zotero-mirror-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_first_read_reads_the_whole_library() {
        let copy = Mirror::empty("0");

        assert!(reads_whole_library(&copy, &copy.plan(&remote(&[("A", 1)]))));
    }

    #[test]
    fn a_few_changes_are_fetched_by_key() {
        let copy = mirror(&[("A", 1), ("B", 2)]);

        assert!(!reads_whole_library(
            &copy,
            &copy.plan(&remote(&[("A", 1), ("B", 9)]))
        ));
    }

    #[test]
    fn many_changes_are_cheaper_to_read_again_by_page() {
        let copy = mirror(&[("A", 1)]);
        let many: Vec<(String, u64)> = (0..=FULL_READ_THRESHOLD)
            .map(|i| (format!("K{i}"), 1))
            .collect();
        let many: HashMap<String, u64> = many.into_iter().collect();

        assert!(reads_whole_library(&copy, &copy.plan(&many)));
    }

    #[test]
    fn a_view_carries_the_works_and_the_version_they_match() {
        let copy = mirror(&[("A", 1), ("B", 2)]);

        let view = copy.view();

        assert_eq!(view.items.len(), 2);
        assert_eq!(view.version, Some(100));
    }

    #[test]
    fn an_unchanged_library_needs_nothing() {
        let copy = mirror(&[("A", 1), ("B", 2)]);

        assert!(copy.plan(&remote(&[("A", 1), ("B", 2)])).is_empty());
    }

    #[test]
    fn new_and_edited_works_are_fetched_and_vanished_ones_removed() {
        let copy = mirror(&[("A", 1), ("B", 2), ("C", 3)]);

        let mut plan = copy.plan(&remote(&[("A", 1), ("B", 7), ("D", 8)]));
        plan.fetch.sort();

        assert_eq!(plan.fetch, vec!["B".to_string(), "D".to_string()]);
        assert_eq!(plan.remove, vec!["C".to_string()]);
    }

    /// Versions only climb within one library, so a work whose version went
    /// *down* is evidence of a different library, not an older edit to keep.
    #[test]
    fn a_version_that_went_backwards_is_fetched_again() {
        let copy = mirror(&[("A", 9)]);

        assert_eq!(copy.plan(&remote(&[("A", 4)])).fetch, vec!["A".to_string()]);
    }

    #[test]
    fn applying_a_plan_leaves_the_copy_equal_to_the_library() {
        let mut copy = mirror(&[("A", 1), ("B", 2), ("C", 3)]);

        copy.apply(
            vec![item("B", 7), item("D", 8)],
            &["C".to_string()],
            Some(8),
        );

        let mut held: Vec<(String, u64)> = copy
            .items
            .iter()
            .map(|i| (i.key.clone(), i.version))
            .collect();
        held.sort();
        assert_eq!(
            held,
            vec![("A".into(), 1), ("B".into(), 7), ("D".into(), 8)]
        );
        assert_eq!(copy.version, Some(8));
    }

    #[test]
    fn the_most_recently_changed_work_comes_first() {
        let copy = mirror(&[("A", 1), ("B", 5), ("C", 3)]);

        let keys: Vec<String> = copy
            .csl()
            .iter()
            .map(|csl| serde_json::from_str::<serde_json::Value>(csl).unwrap()["id"].to_string())
            .collect();

        assert_eq!(keys, vec![r#""B""#, r#""C""#, r#""A""#]);
    }

    #[test]
    fn a_copy_survives_being_written_and_read_back() {
        let dir = scratch("roundtrip");
        let path = path_for(&dir, "0");
        let copy = mirror(&[("A", 1), ("B", 2)]);

        copy.save(&path).unwrap();

        assert_eq!(Mirror::load(&path, "0"), copy);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn no_copy_on_disk_is_an_empty_copy() {
        let dir = scratch("missing");

        let copy = Mirror::load(&path_for(&dir, "0"), "0");

        assert!(copy.items.is_empty());
        assert_eq!(copy.version, None);
    }

    /// A copy that will not parse is thrown away, not repaired: the library is
    /// one read away.
    #[test]
    fn a_corrupt_copy_is_an_empty_copy() {
        let dir = scratch("corrupt");
        let path = path_for(&dir, "0");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();

        assert!(Mirror::load(&path, "0").items.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_copy_of_another_library_is_not_this_ones() {
        let dir = scratch("other");
        let path = path_for(&dir, "0");
        let mut other = mirror(&[("A", 1)]);
        other.library = "9".into();
        other.save(&path).unwrap();

        assert!(Mirror::load(&path, "0").items.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The library id comes from the frontend; it must not be able to name a
    /// file outside the cache directory.
    #[test]
    fn a_library_id_cannot_escape_the_cache_directory() {
        let dir = PathBuf::from("cache");

        let path = path_for(&dir, "../../etc/passwd");

        assert!(path.starts_with(dir.join("zotero")), "{path:?}");
        assert_eq!(path.components().count(), 3, "{path:?}");
    }

    #[test]
    fn an_item_is_read_with_its_key_version_and_compact_csl() {
        let answer = serde_json::json!({
            "key": "37C8RJP8",
            "version": 9756,
            "csljson": "[\n\t{\n\t\t\"id\": \"moore1973\",\n\t\t\"title\": \"Los orígenes\"\n\t}\n]"
        });

        let item = item_from_json(&answer).unwrap();

        assert_eq!(item.key, "37C8RJP8");
        assert_eq!(item.version, 9756);
        assert_eq!(item.csl, r#"{"id":"moore1973","title":"Los orígenes"}"#);
    }

    #[test]
    fn an_item_without_csl_is_skipped_rather_than_guessed() {
        let answer = serde_json::json!({ "key": "37C8RJP8", "version": 1 });

        assert_eq!(item_from_json(&answer), None);
    }
}
