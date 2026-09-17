//! Deciding what a citation renders from (plan-editor.md §11.3).
//!
//! # The rule this exists to keep
//!
//! §11.3 ends with a sentence that governs the whole integration: *"Los fallos
//! de Zotero no deben bloquear la edición ni el guardado."* Someone writing at
//! two in the morning with Zotero closed must still see their citations, still
//! type, and still save.
//!
//! That is possible because a citation carries its own `metadata_snapshot_json`
//! — the CSL-JSON as it was when the citation was made, stored beside the text
//! in `writing_zotero_citations`. The library is an improvement on that
//! snapshot, never a precondition for it.
//!
//! # Why "pending verification" is a state and not a failure
//!
//! Rendering from a snapshot is not degraded output: it is the same CSL-JSON
//! the library would have returned on the day it was cited, so the citation
//! reads correctly. What cannot be known is whether it *still* matches the
//! library. §11.3 asks for that to be marked, and marking it is the whole
//! difference between "we checked and it is right" and "we could not check" —
//! the same distinction the note links and the corpus citations already draw.

use crate::writing::zotero::ZoteroState;

/// Where the CSL-JSON about to be rendered came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationSource {
    /// Fresh from the library, and therefore known to be current.
    Library,
    /// The manuscript's own copy, taken when the citation was made.
    Snapshot,
}

/// What to render, from where, and whether it could be checked.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RenderPlan {
    pub csl_json: String,
    pub source: CitationSource,
    /// True when the library could not confirm this is still current.
    pub pending_verification: bool,
    /// True when the library answered and the work is no longer in it (§11.3,
    /// "biblioteca accesible con item posteriormente eliminado").
    pub missing_from_library: bool,
}

/// Chooses between the library and the snapshot.
///
/// `fresh` is what the library returned for this item, when it was asked and
/// answered. `None` means either that it was not asked — Zotero is unavailable
/// — or that it was asked and does not have the work any more. The Zotero state
/// is what tells those apart, and they are very different things to tell a
/// writer.
pub fn plan(
    snapshot: Option<&str>,
    fresh: Option<&str>,
    state: &ZoteroState,
) -> Option<RenderPlan> {
    let library_answered = matches!(state, ZoteroState::Available);

    if let Some(current) = fresh {
        return Some(RenderPlan {
            csl_json: current.to_string(),
            source: CitationSource::Library,
            pending_verification: false,
            missing_from_library: false,
        });
    }

    // No fresh copy. The snapshot is what the manuscript has, and it is enough
    // to render with — the citation was correct when it was made.
    let held = snapshot?;
    Some(RenderPlan {
        csl_json: held.to_string(),
        source: CitationSource::Snapshot,
        pending_verification: true,
        // The library answered and did not have it. That is a deletion, not an
        // outage, and it is the one case where the writer should be told
        // something about the source rather than about the connection.
        missing_from_library: library_answered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNAPSHOT: &str = r#"{"id":"ABCD1234","type":"book","title":"Il formaggio e i vermi"}"#;
    const FRESH: &str =
        r#"{"id":"ABCD1234","type":"book","title":"Il formaggio e i vermi","edition":"2"}"#;

    /// The sentence the whole module serves: a closed Zotero must not stop
    /// anyone writing. There is always something to render as long as the
    /// manuscript has its snapshot.
    #[test]
    fn a_closed_zotero_still_renders_the_citation() {
        let plan = plan(Some(SNAPSHOT), None, &ZoteroState::EndpointUnavailable).expect("a plan");

        assert_eq!(plan.source, CitationSource::Snapshot);
        assert_eq!(plan.csl_json, SNAPSHOT);
    }

    /// Rendering from a snapshot is not wrong output, it is unverified output.
    /// Saying so is the difference between "checked and right" and "could not
    /// check".
    #[test]
    fn rendering_from_a_snapshot_is_marked_pending_verification() {
        for state in [
            ZoteroState::EndpointUnavailable,
            ZoteroState::ApiDisabled,
            ZoteroState::Timeout,
        ] {
            let plan = plan(Some(SNAPSHOT), None, &state).expect("a plan");
            assert!(
                plan.pending_verification,
                "{state:?} was not marked pending"
            );
        }
    }

    #[test]
    fn a_live_library_supersedes_the_snapshot_and_needs_no_marking() {
        let plan = plan(Some(SNAPSHOT), Some(FRESH), &ZoteroState::Available).expect("a plan");

        assert_eq!(plan.source, CitationSource::Library);
        assert_eq!(plan.csl_json, FRESH);
        assert!(!plan.pending_verification);
        assert!(!plan.missing_from_library);
    }

    /// §11.3's last state: the library is reachable and the work is not in it.
    /// That is a deletion, and the writer should hear about the source rather
    /// than about the connection.
    #[test]
    fn a_work_deleted_from_a_reachable_library_is_reported_as_deleted() {
        let plan = plan(Some(SNAPSHOT), None, &ZoteroState::Available).expect("a plan");

        assert!(plan.missing_from_library);
        assert!(plan.pending_verification);
        assert_eq!(plan.source, CitationSource::Snapshot);
    }

    /// An outage is not a deletion. Telling someone their work was removed from
    /// Zotero because the port was busy would send them looking for a problem
    /// that is not there.
    #[test]
    fn an_outage_is_never_reported_as_a_deletion() {
        for state in [
            ZoteroState::EndpointUnavailable,
            ZoteroState::ApiDisabled,
            ZoteroState::Timeout,
            ZoteroState::InvalidResponse {
                detail: String::new(),
            },
        ] {
            let plan = plan(Some(SNAPSHOT), None, &state).expect("a plan");
            assert!(
                !plan.missing_from_library,
                "{state:?} was reported as a deletion"
            );
        }
    }

    /// A citation with no snapshot and no library has nothing to render. It is
    /// the one case with no answer, and it is reported rather than guessed at.
    #[test]
    fn a_citation_with_neither_has_nothing_to_render() {
        assert_eq!(plan(None, None, &ZoteroState::Available), None);
        assert_eq!(plan(None, None, &ZoteroState::EndpointUnavailable), None);
    }

    /// The library is an improvement on the snapshot, never a precondition: a
    /// fresh copy is used even when the state says something odd, because a
    /// copy that arrived is a copy that arrived.
    #[test]
    fn a_fresh_copy_is_used_whatever_the_state_says_afterwards() {
        let plan = plan(None, Some(FRESH), &ZoteroState::Timeout).expect("a plan");

        assert_eq!(plan.source, CitationSource::Library);
        assert!(!plan.pending_verification);
    }
}
