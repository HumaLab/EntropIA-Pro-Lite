//! Zotero's local API (plan-editor.md §11).
//!
//! Read-only, and deliberately so: §11.1 settles the first integration as
//! `Zotero → EntropIA` only, which removes every question about permissions,
//! sync and corrupting someone's library.
//!
//! # What spike S5 measured, and what it forced
//!
//! The spike ran against a real installation (Zotero 9.0.3, ~5,000 items) and
//! three of its findings are load-bearing here:
//!
//! 1. **`/connector/ping` answers even when the local API is disabled.** That is
//!    what makes an honest diagnosis possible: a liveness probe and a permission
//!    probe are different questions, and §11.3 forbids claiming Zotero is closed
//!    or not installed without evidence. Asking only one of them is how a
//!    disabled API gets reported as a missing program.
//! 2. **A query without `limit` returned 7.9 MB in one response.** So a limit is
//!    not an option here; it is part of every request this module can build.
//! 3. **`Zotero-Server-ID` does not exist in this version.** Instance identity
//!    therefore rests on `Last-Modified-Version`, and §27.7's conservative
//!    policy stops being an alternative and becomes the only one.

pub mod cache;
pub mod connector;
pub mod mirror;

/// Which Zotero library a request addresses (E1c-1: users vs groups).
///
/// The local API routes personal libraries under `/api/users/{id}` and group
/// libraries under `/api/groups/{id}`. There are exactly these two kinds, so
/// anything else fails to parse instead of silently becoming the personal
/// library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryType {
    User,
    Group,
}

impl LibraryType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Group => "group",
        }
    }

    /// The path segment the local API routes on.
    pub fn api_segment(self) -> &'static str {
        match self {
            Self::User => "users",
            Self::Group => "groups",
        }
    }
}

/// A typed Zotero library: its kind plus its id.
///
/// The type system is what keeps an empty id or an unknown kind from
/// silently addressing the wrong library: [`Self::new`] refuses blank ids,
/// and [`LibraryType`] refuses unknown kinds at parse time, so every URL
/// builder below receives an already-valid library.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub library_type: LibraryType,
    pub library_id: String,
}

impl Library {
    /// Fails rather than building a library that would address the wrong path.
    pub fn new(library_type: LibraryType, library_id: impl Into<String>) -> Result<Self, String> {
        let library_id = library_id.into();
        if library_id.trim().is_empty() {
            return Err("the library id must not be empty".to_string());
        }
        Ok(Self {
            library_type,
            library_id,
        })
    }

    pub fn user(library_id: impl Into<String>) -> Self {
        Self::new(LibraryType::User, library_id).expect("a user library id must not be empty")
    }

    pub fn group(library_id: impl Into<String>) -> Self {
        Self::new(LibraryType::Group, library_id).expect("a group library id must not be empty")
    }

    /// The personal default: exactly user/0, unchanged by E1c-1.
    pub fn personal() -> Self {
        Self::user("0")
    }

    /// The per-library key the mirror files and filters on. Byte-identical
    /// to today's bare id for user libraries (so the existing on-disk copy
    /// of `"0"` keeps loading); groups prefix theirs so a group never
    /// shares a file with the user id it mirrors.
    pub fn storage_key(&self) -> String {
        match self.library_type {
            LibraryType::User => self.library_id.clone(),
            LibraryType::Group => format!("group-{}", self.library_id),
        }
    }

    pub fn is_user(&self) -> bool {
        matches!(self.library_type, LibraryType::User)
    }
}

/// Whether the open-in-Zotero command may run (E1c-4).
///
/// Live-verified in the parent's authorized session against group prueba
/// (6680944, item 7EMV3G8H): the select URI opened and the item got selected
/// in Zotero. The command still fails closed with `open_item_disabled` when
/// the gate is off, rather than a Zotero diagnosis.
pub const OPEN_ITEM_ENABLED: bool = true;

/// Builds the official Zotero select URI for one item (E1c-4).
///
/// Personal libraries select via `zotero://select/library/items/{key}`;
/// group libraries via `zotero://select/groups/{id}/items/{key}` — never
/// `user/0`. Strict identity only: `None` unless the key matches Zotero's
/// `^[A-Z0-9]{8}$` shape and the library id is all digits, so CSL ids and
/// unvalidated strings can never be interpolated.
pub fn select_uri(library: &Library, item_key: &str) -> Option<String> {
    if !is_zotero_key(item_key) {
        return None;
    }
    if library.library_id.is_empty()
        || !library.library_id.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    match library.library_type {
        LibraryType::User => Some(format!("zotero://select/library/items/{item_key}")),
        LibraryType::Group => Some(format!(
            "zotero://select/groups/{}/items/{item_key}",
            library.library_id
        )),
    }
}

fn is_zotero_key(item_key: &str) -> bool {
    item_key.len() == 8
        && item_key
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

/// Opens one `zotero://select/…` URI with the OS opener (E1c-4).
///
/// Spawns `explorer` / `open` / `xdg-open` without a shell, mirroring the
/// log-directory opener. Defense in depth: anything that does not start
/// with exactly `zotero://select/` is rejected before spawning, so a future
/// caller cannot turn this into a generic URL opener.
pub fn open_select_uri(uri: &str) -> Result<(), String> {
    if !uri.starts_with("zotero://select/") {
        return Err("refusing to open a non-Zotero select URI".to_string());
    }

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut cmd = std::process::Command::new("explorer");
        cmd.arg(uri);
        cmd
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut cmd = std::process::Command::new("open");
        cmd.arg(uri);
        cmd
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(uri);
        cmd
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not open Zotero item: {error}"))
}

/// What we can honestly say about Zotero right now (§11.3).
///
/// Every variant is something observed. There is deliberately no "Zotero is not
/// installed" and no "Zotero is closed": nothing this module can see
/// distinguishes a closed program from a blocked port, and §11.3 forbids
/// asserting either without evidence.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ZoteroState {
    /// The library answered. Everything works.
    Available,
    /// Nothing answered at all — neither the connector nor the API.
    ///
    /// This says what was observed and nothing more. Zotero may be closed, not
    /// installed, or running with the port blocked; none of those is claimed.
    EndpointUnavailable,
    /// Zotero is there, but the local API is switched off (`403`).
    ///
    /// Only reachable when the connector answered, which is the evidence that
    /// separates this from the case above.
    ApiDisabled,
    /// It answered too slowly. Not the same as absent.
    Timeout,
    /// The library answered that this library does not exist (`404`).
    ///
    /// Per-library only: [`diagnose`] never produces this, and neither does
    /// the probe. It is what lets a typoed id be told apart from an
    /// unreachable Zotero while adding unverified stays allowed.
    NotFound,
    /// It answered with something this build cannot read.
    InvalidResponse { detail: String },
}

/// The probes, and what their pair means.
///
/// Kept as a pure function so the whole diagnosis can be tested without a
/// server, which is the only way to be sure the honest-diagnosis rule holds for
/// every combination rather than the one that happened to be reachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    Answered,
    Forbidden,
    TimedOut,
    Unreachable,
    Malformed,
}

/// Turns the two probes into the one thing we are willing to assert.
pub fn diagnose(connector: ProbeOutcome, library: ProbeOutcome) -> ZoteroState {
    match (connector, library) {
        // The library answered. Nothing else matters.
        (_, ProbeOutcome::Answered) => ZoteroState::Available,

        // The connector answered, so something *is* listening. A refusal from
        // the library is then a permission, not an absence — which is exactly
        // the distinction S5 measured and criterion 12 depends on.
        (ProbeOutcome::Answered, ProbeOutcome::Forbidden) => ZoteroState::ApiDisabled,
        (ProbeOutcome::Answered, ProbeOutcome::TimedOut) => ZoteroState::Timeout,
        (ProbeOutcome::Answered, ProbeOutcome::Malformed) => ZoteroState::InvalidResponse {
            detail: "the library endpoint answered with something unreadable".into(),
        },
        // Listening, but the library endpoint is not there at all.
        (ProbeOutcome::Answered, ProbeOutcome::Unreachable) => ZoteroState::InvalidResponse {
            detail: "the connector answered but the library endpoint did not".into(),
        },

        // A forbidden connector still proves something is listening.
        (ProbeOutcome::Forbidden, ProbeOutcome::Forbidden) => ZoteroState::ApiDisabled,

        // Nothing answered. Say only that.
        (ProbeOutcome::TimedOut, _) => ZoteroState::Timeout,
        _ => ZoteroState::EndpointUnavailable,
    }
}

/// Where a known library came from (E1c-2).
///
/// `personal` is the default the UI offers without evidence; `mirror` is a
/// copy on disk; `catalog` is a row the archive already trusts. The source is
/// what the UI branches on, so it travels with the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KnownLibrarySource {
    Personal,
    Mirror,
    Catalog,
}

/// One library the UI can offer without asking Zotero (E1c-2).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownLibrary {
    pub library_type: LibraryType,
    pub library_id: String,
    pub name: Option<String>,
    pub source: KnownLibrarySource,
}

/// Whether Zotero answers for one library (E1c-2).
///
/// A state, not an error: `unverifiable` keeps adding the library allowed,
/// and only a definitive `404` is [`Self::NotFound`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CheckLibrary {
    Available { version: Option<u64> },
    Unverifiable,
    NotFound,
}

impl CheckLibrary {
    /// Turns one [`connector::library_version`] outcome into the state the UI
    /// branches on. Pure, so every mapping is testable without a server.
    pub fn from_result(result: Result<Option<u64>, ZoteroState>) -> Self {
        match result {
            Ok(version) => Self::Available { version },
            Err(ZoteroState::NotFound) => Self::NotFound,
            Err(_) => Self::Unverifiable,
        }
    }
}

/// Parses a mirror storage key back to its typed library.
///
/// Bare ids are user libraries; `group-{id}` ids are group libraries.
/// Anything else is `None`: the caller skips the file, never fails.
pub fn parse_storage_key(key: &str) -> Option<Library> {
    if let Some(group_id) = key.strip_prefix("group-") {
        Library::new(LibraryType::Group, group_id).ok()
    } else {
        Library::new(LibraryType::User, key).ok()
    }
}

/// Identifies the library a mirror file holds from its filename alone.
///
/// The filename already encodes the storage key, so scanning never reads item
/// payloads. `None` means the file is skipped, never an error.
pub fn library_from_mirror_filename(file_name: &str) -> Option<Library> {
    parse_storage_key(file_name.strip_prefix("library-")?.strip_suffix(".json")?)
}

/// Merges mirror-derived and catalog libraries into the list the UI offers.
///
/// Personal is always first with no name; catalog rows replace mirror-derived
/// entries for the same `(type, id)`; the rest sort by type then id ascending
/// (`"group"` before `"user"`). Pure, so merge and order are testable
/// without a cache directory or a database.
pub fn merge_known_libraries(
    mirror: Vec<Library>,
    catalog: Vec<(Library, String)>,
) -> Vec<KnownLibrary> {
    let mut by_key: std::collections::BTreeMap<(String, String), KnownLibrary> =
        std::collections::BTreeMap::new();
    for library in mirror {
        by_key
            .entry((
                library.library_type.as_str().to_string(),
                library.library_id.clone(),
            ))
            .or_insert(KnownLibrary {
                library_type: library.library_type,
                library_id: library.library_id,
                name: None,
                source: KnownLibrarySource::Mirror,
            });
    }
    for (library, name) in catalog {
        by_key.insert(
            (
                library.library_type.as_str().to_string(),
                library.library_id.clone(),
            ),
            KnownLibrary {
                library_type: library.library_type,
                library_id: library.library_id,
                name: Some(name),
                source: KnownLibrarySource::Catalog,
            },
        );
    }
    // Personal wins over every other source: it is always offered as personal
    // with no name, even when the mirror or the catalog also names user/0.
    by_key.remove(&("user".to_string(), "0".to_string()));
    let mut known = Vec::with_capacity(by_key.len() + 1);
    known.push(KnownLibrary {
        library_type: LibraryType::User,
        library_id: "0".to_string(),
        name: None,
        source: KnownLibrarySource::Personal,
    });
    known.extend(by_key.into_values());
    known
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule §11.3 states and criterion 12 checks: never assert that Zotero
    /// is closed or missing without evidence. These walk every pair so the rule
    /// holds for all of them, not just the one that was easy to reproduce.
    #[test]
    fn a_library_that_answers_is_all_the_evidence_needed() {
        for connector in [
            ProbeOutcome::Answered,
            ProbeOutcome::Forbidden,
            ProbeOutcome::TimedOut,
            ProbeOutcome::Unreachable,
            ProbeOutcome::Malformed,
        ] {
            assert_eq!(
                diagnose(connector, ProbeOutcome::Answered),
                ZoteroState::Available,
                "connector {connector:?} should not override a working library"
            );
        }
    }

    /// S5's central finding: the connector answers even when the API is off, so
    /// a refusal alongside a live connector is a permission and not an absence.
    #[test]
    fn a_live_connector_turns_a_refusal_into_a_permission_problem() {
        assert_eq!(
            diagnose(ProbeOutcome::Answered, ProbeOutcome::Forbidden),
            ZoteroState::ApiDisabled
        );
    }

    #[test]
    fn nothing_answering_is_reported_as_nothing_answering() {
        assert_eq!(
            diagnose(ProbeOutcome::Unreachable, ProbeOutcome::Unreachable),
            ZoteroState::EndpointUnavailable
        );
    }

    /// Slow is not absent, and reporting it as absent would send someone to
    /// reinstall a program that is running.
    #[test]
    fn a_timeout_is_not_an_absence() {
        assert_eq!(
            diagnose(ProbeOutcome::TimedOut, ProbeOutcome::TimedOut),
            ZoteroState::Timeout
        );
        assert_eq!(
            diagnose(ProbeOutcome::Answered, ProbeOutcome::TimedOut),
            ZoteroState::Timeout
        );
    }

    #[test]
    fn a_reachable_connector_with_an_unreadable_library_is_neither_absent_nor_disabled() {
        let state = diagnose(ProbeOutcome::Answered, ProbeOutcome::Malformed);
        assert!(matches!(state, ZoteroState::InvalidResponse { .. }));
    }

    /// The vocabulary itself is the guarantee. If someone adds a variant that
    /// asserts something unobservable, this is where the review happens.
    /// `NotFound` is the one per-library variant: a definitive 404 for the
    /// requested library, never a claim about Zotero itself.
    #[test]
    fn the_vocabulary_asserts_nothing_it_cannot_observe() {
        let observable = [
            ZoteroState::Available,
            ZoteroState::EndpointUnavailable,
            ZoteroState::ApiDisabled,
            ZoteroState::Timeout,
            ZoteroState::NotFound,
            ZoteroState::InvalidResponse {
                detail: String::new(),
            },
        ];
        // Six states, none of which names a cause we cannot see.
        assert_eq!(observable.len(), 6);
    }

    /// E1c-1 RED: the seam names exactly two library kinds; anything else
    /// must fail to parse rather than silently becoming the personal library.
    #[test]
    fn e1c1_library_type_parses_user_and_group_but_rejects_unknown() {
        let user: super::Library =
            serde_json::from_value(serde_json::json!({"libraryType": "user", "libraryId": "0"}))
                .unwrap();
        assert_eq!(user.library_type, super::LibraryType::User);
        let group: super::Library = serde_json::from_value(
            serde_json::json!({"libraryType": "group", "libraryId": "6680944"}),
        )
        .unwrap();
        assert_eq!(group.library_type, super::LibraryType::Group);
        assert!(serde_json::from_value::<super::Library>(
            serde_json::json!({"libraryType": "team", "libraryId": "0"})
        )
        .is_err());
    }

    /// E1c-1 RED: an empty id must never become a URL for the wrong library.
    #[test]
    fn e1c1_empty_library_id_is_rejected() {
        assert!(super::Library::new(super::LibraryType::User, "").is_err());
        assert!(super::Library::new(super::LibraryType::Group, "   ").is_err());
    }

    /// E1c-1 RED: user libraries keep today's on-disk key (`"0"` stays `"0"`).
    #[test]
    fn e1c1_user_storage_key_is_the_id_itself() {
        assert_eq!(super::Library::user("0").storage_key(), "0");
    }

    /// E1c-1 RED: group libraries use a deterministic key that cannot
    /// collide with a user library holding the same id.
    #[test]
    fn e1c1_group_storage_key_is_prefixed_and_collision_free() {
        let group = super::Library::group("6680944");
        assert_eq!(group.storage_key(), "group-6680944");
        assert_ne!(
            group.storage_key(),
            super::Library::user("6680944").storage_key()
        );
    }

    /// E1c-1 RED: the personal default is exactly user/0.
    #[test]
    fn e1c1_personal_default_is_user_zero() {
        assert_eq!(super::Library::personal(), super::Library::user("0"));
    }

    /// E1c-2 RED: storage keys parse back to typed libraries; anything else
    /// is skipped, never an error.
    #[test]
    fn e1c2_storage_key_parses_back_to_typed_libraries() {
        assert_eq!(parse_storage_key("0"), Some(super::Library::user("0")));
        assert_eq!(
            parse_storage_key("group-6680944"),
            Some(super::Library::group("6680944"))
        );
        assert_eq!(parse_storage_key(""), None);
        assert_eq!(parse_storage_key("group-"), None);
    }

    /// E1c-2 RED: the mirror filename alone identifies the library, so
    /// scanning never reads item payloads.
    #[test]
    fn e1c2_mirror_filename_identifies_the_library() {
        assert_eq!(
            library_from_mirror_filename("library-group-6680944.json"),
            Some(super::Library::group("6680944"))
        );
        assert_eq!(
            library_from_mirror_filename("library-0.json"),
            Some(super::Library::user("0"))
        );
        assert_eq!(library_from_mirror_filename("library-.json"), None);
        assert_eq!(library_from_mirror_filename("notes.txt"), None);
        assert_eq!(library_from_mirror_filename("library-0.json.partial"), None);
    }

    /// E1c-2 RED: personal first, catalog wins over mirror for the same
    /// library, deterministic type/id order after personal.
    #[test]
    fn e1c2_known_libraries_merge_catalog_over_mirror_in_order() {
        let merged = merge_known_libraries(
            vec![
                super::Library::user("0"),
                super::Library::group("6680944"),
                super::Library::user("123"),
            ],
            vec![(super::Library::group("6680944"), "Test Group".to_string())],
        );

        assert_eq!(
            merged,
            vec![
                KnownLibrary {
                    library_type: super::LibraryType::User,
                    library_id: "0".to_string(),
                    name: None,
                    source: KnownLibrarySource::Personal,
                },
                KnownLibrary {
                    library_type: super::LibraryType::Group,
                    library_id: "6680944".to_string(),
                    name: Some("Test Group".to_string()),
                    source: KnownLibrarySource::Catalog,
                },
                KnownLibrary {
                    library_type: super::LibraryType::User,
                    library_id: "123".to_string(),
                    name: None,
                    source: KnownLibrarySource::Mirror,
                },
            ]
        );
    }

    /// E1c-2 TRIANGULATE: the wire shape the UI half will consume — camelCase
    /// libraries with a lowercase source, tagged check states.
    #[test]
    fn e1c2_wire_shapes_are_camel_case_and_tagged() {
        let library = serde_json::to_value(KnownLibrary {
            library_type: super::LibraryType::Group,
            library_id: "6680944".to_string(),
            name: Some("Test Group".to_string()),
            source: KnownLibrarySource::Catalog,
        })
        .unwrap();
        assert_eq!(
            library,
            serde_json::json!({
                "libraryType": "group",
                "libraryId": "6680944",
                "name": "Test Group",
                "source": "catalog",
            })
        );
        assert_eq!(
            serde_json::to_value(CheckLibrary::Available { version: Some(7) }).unwrap(),
            serde_json::json!({ "status": "available", "version": 7 })
        );
        assert_eq!(
            serde_json::to_value(CheckLibrary::NotFound).unwrap(),
            serde_json::json!({ "status": "not_found" })
        );
        assert_eq!(
            serde_json::to_value(CheckLibrary::Unverifiable).unwrap(),
            serde_json::json!({ "status": "unverifiable" })
        );
    }

    /// E1c-2 RED: the check is a state, never an error; only a definitive
    /// 404 is `not_found`, everything else unreachable is `unverifiable`.
    #[test]
    fn e1c2_check_library_maps_connector_outcome_to_state() {
        assert_eq!(
            CheckLibrary::from_result(Ok(Some(7))),
            CheckLibrary::Available { version: Some(7) }
        );
        assert_eq!(
            CheckLibrary::from_result(Ok(None)),
            CheckLibrary::Available { version: None }
        );
        assert_eq!(
            CheckLibrary::from_result(Err(ZoteroState::NotFound)),
            CheckLibrary::NotFound
        );
        for state in [
            ZoteroState::ApiDisabled,
            ZoteroState::Timeout,
            ZoteroState::EndpointUnavailable,
            ZoteroState::InvalidResponse {
                detail: "the library answered 500".to_string(),
            },
        ] {
            assert_eq!(
                CheckLibrary::from_result(Err(state)),
                CheckLibrary::Unverifiable,
                "unreachable Zotero must stay addable"
            );
        }
    }

    /// E1c-4 RED: personal libraries select via `library/items` (the official
    /// Zotero select scheme, never `user/0`).
    #[test]
    fn e1c4_personal_select_uri_uses_library_items() {
        let library = super::Library::user("0");
        assert_eq!(
            super::select_uri(&library, "7EMV3G8H"),
            Some("zotero://select/library/items/7EMV3G8H".to_string())
        );
    }

    /// E1c-4 RED: group libraries select via `groups/{id}/items`.
    #[test]
    fn e1c4_group_select_uri_uses_groups_id_items() {
        let library = super::Library::group("6680944");
        assert_eq!(
            super::select_uri(&library, "7EMV3G8H"),
            Some("zotero://select/groups/6680944/items/7EMV3G8H".to_string())
        );
    }

    /// E1c-4 RED: strict identity only — anything that is not an 8-char
    /// uppercase-alphanumeric key or an all-digit library id yields None, so
    /// CSL ids and unvalidated strings can never be interpolated.
    #[test]
    fn e1c4_select_uri_rejects_invalid_keys_and_ids() {
        let personal = super::Library::user("0");
        for bad_key in [
            "",
            "   ",
            "abc123",
            "7EMV3G8H ",
            "7emv3g8h",
            "TOOLONGKEY1",
            "ABCD-EFG",
            "http://x",
        ] {
            assert_eq!(
                super::select_uri(&personal, bad_key),
                None,
                "key {bad_key:?} must not build a URI"
            );
        }
        let group = super::Library::group("6680944");
        assert_eq!(super::select_uri(&group, "7EMV3G8H "), None);
        // library_id values that fail the all-digits check never interpolate,
        // even when constructed outside `Library::new`.
        let evil_group = super::Library {
            library_type: super::LibraryType::Group,
            library_id: "6680944/../x".to_string(),
        };
        assert_eq!(super::select_uri(&evil_group, "7EMV3G8H"), None);
        let evil_user = super::Library {
            library_type: super::LibraryType::User,
            library_id: "".to_string(),
        };
        assert_eq!(super::select_uri(&evil_user, "7EMV3G8H"), None);
    }

    /// E1c-4 RED: defense in depth — the opener rejects anything that does
    /// not start with exactly `zotero://select/`, before spawning anything.
    #[test]
    fn e1c4_opener_rejects_non_zotero_uris() {
        for bad in [
            "https://example.com",
            "http://localhost:23119/api/users/0/items",
            "zotero://search/abc",
            "zotero://selectivity/items/7EMV3G8H",
            " zotero://select/library/items/7EMV3G8H",
            "ZOTERO://select/library/items/7EMV3G8H",
            "",
        ] {
            assert!(
                super::open_select_uri(bad).is_err(),
                "uri {bad:?} must be rejected without spawning"
            );
        }
    }

    /// E1c-4: gate on after the 2026 live verification against group prueba
    /// (6680944, item 7EMV3G8H) — the authorized session opened the select URI
    /// and the user confirmed the item got selected in Zotero.
    #[test]
    // The gate is a constant on purpose: this test pins its value, so the
    // constant assertion is the point, not an accident.
    #[allow(clippy::assertions_on_constants)]
    fn e1c4_open_item_is_enabled_after_live_verification() {
        assert!(
            super::OPEN_ITEM_ENABLED,
            "gate must be on after live verification"
        );
    }

    /// E1c-4 live probe against group prueba (6680944, fixture 7EMV3G8H).
    /// Run ONLY during the parent's authorized live session with Zotero open:
    /// `cargo test -p ... live_open_select_prueba_fixture -- --ignored --nocapture`.
    /// Never runs in normal verification (it spawns the OS opener).
    #[test]
    #[ignore]
    fn live_open_select_prueba_fixture() {
        let library = super::Library::group("6680944");
        let uri = super::select_uri(&library, "7EMV3G8H")
            .expect("prueba fixture must build a select URI");
        assert_eq!(uri, "zotero://select/groups/6680944/items/7EMV3G8H");
        super::open_select_uri(&uri).expect("live Zotero must open the fixture item");
    }
}
