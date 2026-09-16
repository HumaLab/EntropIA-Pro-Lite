//! What the agent can actually do here (plan-editor.md §14.1, Unit 7, gap G9).
//!
//! # Why this is published rather than discovered
//!
//! Unit 7 asks for the real capability matrix to be published **before** any
//! action is exposed, and for what is unavailable to be shown as such. The
//! reason is in G9: `ClienteLlm` is a trait and its provider can be swapped at
//! the call site, but `ClienteEmbeddings` and `ClienteRerank` are concrete
//! structs bound to OpenRouter. So every action that needs retrieval — finding
//! evidence in the corpus, finding contradictions, recalling related notes —
//! cannot be offered without changing the `entropia-agent` crate, whatever
//! model the writer has configured.
//!
//! Offering them anyway and failing at the moment of use would be the worst of
//! both: the writer has already selected a passage and formed an intention.
//! Saying so up front costs nothing and is true.
//!
//! # And none of this blocks writing
//!
//! Unit 7 is explicit that no absence blocks manual editing. Every action here
//! being unavailable is an ordinary state of the application, not a fault: the
//! manuscript is written by a person, and the agent is an accessory to that.

/// What an action needs in order to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Requirement {
    /// A chat model. Swappable, because `ClienteLlm` is a trait.
    Chat,
    /// Retrieval over the corpus: embeddings and reranking. Bound to
    /// OpenRouter by concrete types (G9).
    Retrieval,
}

/// One action, and the plain truth about whether it can run.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AgentAction {
    /// Stable identifier, used as `action_type` on a suggestion.
    pub id: &'static str,
    pub requires: Requirement,
    pub available: bool,
    /// Why not, in words a writer can act on. Empty when it is available.
    pub unavailable_reason: &'static str,
}

/// The actions §14.1 lists, each against what it needs.
const ACTIONS: [(&str, Requirement); 14] = [
    ("fix_spelling", Requirement::Chat),
    ("review_wording", Requirement::Chat),
    ("improve_clarity", Requirement::Chat),
    ("improve_argument", Requirement::Chat),
    ("shorten", Requirement::Chat),
    ("expand", Requirement::Chat),
    ("summarise", Requirement::Chat),
    ("rephrase", Requirement::Chat),
    ("detect_repetition", Requirement::Chat),
    ("detect_contradiction", Requirement::Chat),
    // Everything below needs retrieval, and retrieval is what G9 pins down.
    ("find_evidence", Requirement::Retrieval),
    ("find_counter_evidence", Requirement::Retrieval),
    ("find_counterexamples", Requirement::Retrieval),
    ("recall_notes", Requirement::Retrieval),
];

const NO_CHAT: &str = "no_chat_model";
const NO_RETRIEVAL: &str = "retrieval_unavailable";

/// The matrix, for the chat and retrieval this build actually has.
///
/// Both answers are passed in rather than guessed at here: whether a chat model
/// is configured is a question for the settings, and whether retrieval works is
/// a question about the agent crate. Deciding either from inside this function
/// would be inventing a fact.
pub fn matrix(has_chat: bool, has_retrieval: bool) -> Vec<AgentAction> {
    ACTIONS
        .iter()
        .map(|(id, requires)| {
            let available = match requires {
                Requirement::Chat => has_chat,
                // Retrieval needs a model to reason with as well as the
                // retrieval itself; an action that can find evidence and not
                // discuss it is not one of the actions §14.1 lists.
                Requirement::Retrieval => has_chat && has_retrieval,
            };
            AgentAction {
                id,
                requires: *requires,
                available,
                unavailable_reason: if available {
                    ""
                } else if !has_chat {
                    NO_CHAT
                } else {
                    NO_RETRIEVAL
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(matrix: &'a [AgentAction], id: &str) -> &'a AgentAction {
        matrix.iter().find(|action| action.id == id).expect(id)
    }

    /// G9: retrieval is bound to OpenRouter by concrete types, so an action
    /// that depends on it is not available merely because a chat model is.
    /// Offering it and failing at the moment of use is the worst outcome — the
    /// writer has already selected a passage and formed an intention.
    #[test]
    fn retrieval_actions_need_more_than_a_chat_model() {
        let matrix = matrix(true, false);

        assert!(find(&matrix, "improve_clarity").available);
        assert!(!find(&matrix, "find_evidence").available);
        assert_eq!(find(&matrix, "find_evidence").unavailable_reason, NO_RETRIEVAL);
    }

    #[test]
    fn everything_is_available_when_both_are() {
        let matrix = matrix(true, true);

        assert!(matrix.iter().all(|action| action.available), "{matrix:?}");
        assert!(matrix.iter().all(|action| action.unavailable_reason.is_empty()));
    }

    /// With no model at all, nothing runs — and the reason names the model
    /// rather than the retrieval, because that is the one to fix first.
    #[test]
    fn without_a_model_nothing_runs_and_the_reason_says_which() {
        let matrix = matrix(false, true);

        assert!(matrix.iter().all(|action| !action.available));
        assert!(matrix
            .iter()
            .all(|action| action.unavailable_reason == NO_CHAT));
    }

    /// An action that could find evidence but not discuss it is not one of the
    /// actions §14.1 lists.
    #[test]
    fn retrieval_without_a_model_is_not_an_action() {
        assert!(!find(&matrix(false, true), "find_evidence").available);
    }

    /// The matrix is the published contract, so its size and shape are part of
    /// it: an action added without a decision about what it needs shows up here.
    #[test]
    fn every_action_of_14_1_is_accounted_for() {
        let matrix = matrix(true, true);

        assert_eq!(matrix.len(), 14);
        let retrieval = matrix
            .iter()
            .filter(|a| a.requires == Requirement::Retrieval)
            .count();
        assert_eq!(retrieval, 4, "the retrieval-bound set changed: {matrix:?}");
    }

    /// Unavailability is a state, never a silence: an action that cannot run
    /// still appears, with a reason.
    #[test]
    fn an_unavailable_action_is_shown_rather_than_hidden() {
        let matrix = matrix(false, false);

        assert_eq!(matrix.len(), 14);
        assert!(matrix
            .iter()
            .all(|action| !action.unavailable_reason.is_empty()));
    }
}
