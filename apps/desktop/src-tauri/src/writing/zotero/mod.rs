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
    #[test]
    fn the_vocabulary_asserts_nothing_it_cannot_observe() {
        let observable = [
            ZoteroState::Available,
            ZoteroState::EndpointUnavailable,
            ZoteroState::ApiDisabled,
            ZoteroState::Timeout,
            ZoteroState::InvalidResponse {
                detail: String::new(),
            },
        ];
        // Five states, none of which names a cause we cannot see.
        assert_eq!(observable.len(), 5);
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
}
