//! A regenerable cache of Zotero items (plan-editor.md §11.2, §27.7).
//!
//! # It is a cache, and nothing depends on it surviving
//!
//! Every entry here can be fetched again. That is what makes throwing the whole
//! thing away a safe answer to any doubt, and it is why the rules below can
//! afford to be as conservative as they are: the cost of being wrong in the
//! careful direction is one more HTTP request.
//!
//! What must *never* be thrown away is the manuscript's own snapshots. A
//! citation carries its `metadata_snapshot_json`, and that is the writer's,
//! stored beside their text in `writing_zotero_citations`. This module cannot
//! reach it — it holds library items keyed by their Zotero key and knows
//! nothing about any document — so "invalidating the cache" and "losing a
//! citation" are not even expressible as the same operation.
//!
//! # Why a key match is not an identity
//!
//! This is the sharp rule of §11.2, and the one worth reading twice.
//!
//! Zotero item keys are eight characters, unique *within a library*. Point
//! EntropIA at a different Zotero profile and `ABCD1234` there is a different
//! work — very probably someone else's. A cache that answered "what is
//! `ABCD1234`?" without asking "in which library?" would silently re-point a
//! citation at a book its author never read, and nothing about the manuscript
//! would look wrong.
//!
//! So [`ZoteroCache::get`] takes the instance being asked about. A caller
//! cannot look anything up without saying which library it believes it is
//! talking to, which makes re-linking by key match a thing that cannot be
//! written rather than a rule someone has to remember.
//!
//! # What identifies an instance, and why it is so weak
//!
//! Spike S5 measured that `Zotero-Server-ID` does not exist in Zotero 9.0.3.
//! All that is left is `Last-Modified-Version`, which is monotonic *within* a
//! library and meaningless across them — two unrelated libraries can both sit
//! at version 500. §27.7's conservative policy therefore stops being one option
//! among several: when sameness cannot be proven, the cache assumes the
//! instance changed.

use std::collections::HashMap;

/// Which library this is, as well as can be told.
///
/// `origin` is the address and library id — a different one is certainly a
/// different instance. `version` is `Last-Modified-Version`, which can only
/// ever *disprove* sameness, never establish it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InstanceFingerprint {
    pub origin: String,
    pub version: Option<u64>,
}

impl InstanceFingerprint {
    pub fn new(origin: impl Into<String>, version: Option<u64>) -> Self {
        Self {
            origin: origin.into(),
            version,
        }
    }
}

/// What observing the library again means for what is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheVerdict {
    /// Same library, same version. What is held still describes it.
    Unchanged,
    /// Same library, but it has moved on. The entries are stale, not wrong
    /// about *which* library — they are simply out of date.
    Stale,
    /// A different library, or one that cannot be shown to be the same. Nothing
    /// held can be trusted to describe it, including the keys.
    InstanceChanged,
}

/// Zotero items, held only for as long as they can be trusted.
#[derive(Debug, Clone, Default)]
pub struct ZoteroCache {
    fingerprint: Option<InstanceFingerprint>,
    entries: HashMap<String, String>,
}

impl ZoteroCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// What the cache currently believes it is holding items from.
    pub fn instance(&self) -> Option<&InstanceFingerprint> {
        self.fingerprint.as_ref()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Records what the library looks like now, and clears what that invalidates.
    ///
    /// A version that went *backwards* is the clearest possible evidence of a
    /// different library: `Last-Modified-Version` only ever climbs within one.
    /// A version that is unknown on either side proves nothing, and proving
    /// nothing is treated as a change (§27.7).
    pub fn observe(&mut self, seen: InstanceFingerprint) -> CacheVerdict {
        let verdict = match self.fingerprint.as_ref() {
            None => CacheVerdict::InstanceChanged,
            Some(held) if held.origin != seen.origin => CacheVerdict::InstanceChanged,
            Some(held) => match (held.version, seen.version) {
                (Some(before), Some(now)) if now == before => CacheVerdict::Unchanged,
                (Some(before), Some(now)) if now > before => CacheVerdict::Stale,
                // Backwards, or unknown on either side. Neither can be shown to
                // be the same library, so neither is treated as one.
                _ => CacheVerdict::InstanceChanged,
            },
        };

        if verdict != CacheVerdict::Unchanged {
            self.entries.clear();
        }
        self.fingerprint = Some(seen);
        verdict
    }

    /// Stores an item, under the instance the cache is currently holding.
    ///
    /// Refuses when no instance has been observed: an item with no library
    /// behind it is the thing this module exists to prevent.
    pub fn put(&mut self, key: impl Into<String>, csl_json: impl Into<String>) -> bool {
        if self.fingerprint.is_none() {
            return false;
        }
        self.entries.insert(key.into(), csl_json.into());
        true
    }

    /// The item, **only** if the caller is asking about the library it came from.
    ///
    /// The fingerprint is a parameter rather than something the cache assumes,
    /// so there is no way to ask "what is `ABCD1234`?" without also saying
    /// which library. That is the whole defence against re-linking a citation
    /// to a different work that happens to share a key.
    pub fn get(&self, asking_about: &InstanceFingerprint, key: &str) -> Option<&str> {
        let held = self.fingerprint.as_ref()?;
        // Only the origin is compared. A newer version is stale, not wrong
        // about which library this is — and answering from a slightly old copy
        // of the right library beats refusing to answer at all.
        if held.origin != asking_about.origin {
            return None;
        }
        self.entries.get(key).map(String::as_str)
    }

    /// Throws everything away. Always safe: every entry can be fetched again.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITEM: &str = r#"{"id":"ABCD1234","type":"book","title":"Il formaggio e i vermi"}"#;
    const OTHER: &str =
        r#"{"id":"ABCD1234","type":"book","title":"Un libro completamente distinto"}"#;

    fn library(version: u64) -> InstanceFingerprint {
        InstanceFingerprint::new("http://localhost:23119/users/0", Some(version))
    }

    fn other_library(version: u64) -> InstanceFingerprint {
        InstanceFingerprint::new("http://localhost:23119/users/9", Some(version))
    }

    fn cache_at(version: u64) -> ZoteroCache {
        let mut cache = ZoteroCache::new();
        cache.observe(library(version));
        cache.put("ABCD1234", ITEM);
        cache
    }

    /// The rule §11.2 is built around, stated as a test: the same key in a
    /// different library is a different work, and the cache must not confuse
    /// them even when the key matches exactly.
    #[test]
    fn a_key_that_matches_in_another_library_is_not_the_same_item() {
        let cache = cache_at(100);

        assert_eq!(cache.get(&other_library(100), "ABCD1234"), None);
        assert_eq!(cache.get(&library(100), "ABCD1234"), Some(ITEM));
    }

    /// The defence is structural: a lookup names the library it is asking
    /// about, so the mistake is not available to make.
    #[test]
    fn nothing_can_be_looked_up_without_naming_a_library() {
        let mut cache = ZoteroCache::new();

        // No instance observed yet, so there is nothing to put an item under.
        assert!(!cache.put("ABCD1234", ITEM));
        assert!(cache.is_empty());
    }

    #[test]
    fn a_library_at_the_same_version_is_still_the_one_that_was_cached() {
        let mut cache = cache_at(100);

        assert_eq!(cache.observe(library(100)), CacheVerdict::Unchanged);
        assert_eq!(cache.get(&library(100), "ABCD1234"), Some(ITEM));
    }

    /// A version that climbed means the same library changed. The entries are
    /// out of date, so they go — fetching them again is one request.
    #[test]
    fn a_library_that_moved_on_leaves_its_entries_behind() {
        let mut cache = cache_at(100);

        assert_eq!(cache.observe(library(140)), CacheVerdict::Stale);
        assert!(cache.is_empty());
    }

    /// `Last-Modified-Version` only climbs within one library, so a version
    /// that went backwards is the clearest evidence available of a different
    /// one.
    #[test]
    fn a_version_that_went_backwards_is_a_different_library() {
        let mut cache = cache_at(140);

        assert_eq!(cache.observe(library(100)), CacheVerdict::InstanceChanged);
        assert!(cache.is_empty());
    }

    #[test]
    fn a_different_origin_is_a_different_library_whatever_the_version() {
        let mut cache = cache_at(100);

        assert_eq!(
            cache.observe(other_library(100)),
            CacheVerdict::InstanceChanged
        );
        assert!(cache.is_empty());
    }

    /// §27.7's conservative policy: S5 removed `Zotero-Server-ID`, so an
    /// unknown version proves nothing — and proving nothing is treated as a
    /// change rather than as sameness.
    #[test]
    fn a_version_that_cannot_be_read_is_treated_as_a_change() {
        let mut cache = cache_at(100);
        assert_eq!(
            cache.observe(InstanceFingerprint::new(
                "http://localhost:23119/users/0",
                None
            )),
            CacheVerdict::InstanceChanged
        );

        let mut unknown = ZoteroCache::new();
        unknown.observe(InstanceFingerprint::new(
            "http://localhost:23119/users/0",
            None,
        ));
        unknown.put("ABCD1234", ITEM);
        assert_eq!(unknown.observe(library(100)), CacheVerdict::InstanceChanged);
        assert!(unknown.is_empty());
    }

    /// Switching libraries and back must not resurrect the first one's items
    /// under the second one's keys.
    #[test]
    fn items_do_not_survive_a_round_trip_through_another_library() {
        let mut cache = cache_at(100);

        cache.observe(other_library(50));
        cache.put("ABCD1234", OTHER);
        cache.observe(library(100));

        assert_eq!(cache.get(&library(100), "ABCD1234"), None);
    }

    /// The other half of §11.2: invalidating the cache is not losing a
    /// citation. The manuscript's snapshot is the caller's own data and this
    /// module has no way to touch it — which is why the test can hold it
    /// through every invalidation above.
    #[test]
    fn invalidating_the_cache_leaves_the_manuscripts_snapshot_alone() {
        let manuscript_snapshot = ITEM.to_string();
        let mut cache = cache_at(100);

        cache.observe(other_library(1));
        cache.observe(library(999));
        cache.clear();

        assert!(cache.is_empty());
        assert_eq!(
            manuscript_snapshot, ITEM,
            "the snapshot was not the cache's to lose"
        );
    }

    /// Throwing everything away is always available, because everything here
    /// can be fetched again.
    #[test]
    fn everything_can_be_thrown_away_at_any_time() {
        let mut cache = cache_at(100);
        assert_eq!(cache.len(), 1);

        cache.clear();

        assert!(cache.is_empty());
        // The instance is still known, so the next fetch refills rather than
        // starting from nothing.
        assert_eq!(
            cache.instance().map(|i| i.origin.as_str()),
            Some("http://localhost:23119/users/0")
        );
    }

    /// A slightly old copy of the right library is a better answer than no
    /// answer: the version is what makes it stale, not what makes it wrong.
    #[test]
    fn a_lookup_tolerates_a_version_it_has_not_caught_up_with() {
        let cache = cache_at(100);

        assert_eq!(cache.get(&library(140), "ABCD1234"), Some(ITEM));
    }
}
