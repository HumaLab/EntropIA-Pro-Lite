//! Placing a citation's prefix and suffix (plan-editor.md §11.5).
//!
//! # Why this exists at all
//!
//! Spike S3 measured that hayagriva has no per-item affix field: `CitationItem`
//! exposes exactly `entry`, `locator`, `locale`, `hidden` and `purpose`, and
//! `CitationRequest::prefix()`/`suffix()` are private and read the *style's*
//! brackets rather than a writer's words. The rendered tree is public and
//! mutable, though, so the affixes are injected after rendering.
//!
//! S3 was explicit about what that means: **this is our code doing CSL's job**,
//! and it has to be treated as such — a small tested helper with a case per
//! style class, not an incidental string patch. That is what this file is.
//!
//! # The rule
//!
//! A bracket style renders `(Ginzburg, 1976, p. 45)`, and CSL requires the
//! affixes *inside* those brackets: `(see Ginzburg, 1976, p. 45, and passim)`.
//! A note style renders `Carlo Ginzburg, … (Einaudi, 1976), 45.` with no
//! brackets to go inside, so the affixes wrap the whole thing — and the style's
//! terminal period has to be moved out of the way first, or the suffix lands
//! after it and reads as a new sentence.
//!
//! The logic is kept over plain segments rather than hayagriva's element tree,
//! so the rule is tested on its own and only the adapter depends on the
//! library's types.

/// How a style wraps its citations, which decides where an affix belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleShape {
    /// `(Ginzburg, 1976)` — affixes go inside the brackets.
    Bracketed,
    /// `Carlo Ginzburg, … 45.` — affixes wrap the whole note.
    Note,
}

/// Recognises the shape from the rendered text, so a style this build has never
/// heard of is still handled by what it produced rather than by its name.
pub fn shape_of(rendered: &str) -> StyleShape {
    let trimmed = rendered.trim();
    // Either pair of brackets, one condition: the two used to be separate arms
    // returning the same shape, which said "these are different cases" about
    // two spellings of the same one.
    let bracketed = (trimmed.starts_with('(') && trimmed.ends_with(')'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'));

    if bracketed {
        StyleShape::Bracketed
    } else {
        StyleShape::Note
    }
}

/// Punctuation a note style puts at the very end of its own rendering.
///
/// Trimmed before a suffix is appended so the suffix continues the sentence
/// rather than starting a new one after the period. S3 recorded the untrimmed
/// version — `45., and passim` — as a defect of this wrapper.
fn trim_terminal(text: &str) -> &str {
    text.trim_end().trim_end_matches(['.', ',', ';'])
}

/// Puts `prefix` and `suffix` where the style's shape requires.
///
/// Both are optional and empty ones change nothing, so a citation with no
/// affixes renders exactly as the engine produced it — this helper is never in
/// the way of the ordinary case.
pub fn apply_affixes(rendered: &str, prefix: &str, suffix: &str) -> String {
    let prefix = prefix.trim();
    let suffix = suffix.trim();
    if prefix.is_empty() && suffix.is_empty() {
        return rendered.to_string();
    }

    match shape_of(rendered) {
        StyleShape::Bracketed => {
            let trimmed = rendered.trim();
            // Keep the style's own brackets: they are the style's, not ours,
            // and rebuilding them would silently normalise `[1]` into `(1)`.
            let open = &trimmed[..1];
            let close = &trimmed[trimmed.len() - 1..];
            let inner = &trimmed[1..trimmed.len() - 1];

            let mut out = String::from(open);
            if !prefix.is_empty() {
                out.push_str(prefix);
                out.push(' ');
            }
            out.push_str(inner);
            if !suffix.is_empty() {
                out.push_str(", ");
                out.push_str(suffix);
            }
            out.push_str(close);
            out
        }
        StyleShape::Note => {
            let body = if suffix.is_empty() {
                rendered.trim().to_string()
            } else {
                // The note's terminal period would otherwise leave the suffix
                // reading as a sentence of its own.
                format!("{}, {}", trim_terminal(rendered), suffix)
            };
            if prefix.is_empty() {
                body
            } else {
                format!("{prefix} {body}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ordinary case is the one that must not be disturbed: a citation with
    /// no affixes has to come out exactly as the engine rendered it.
    #[test]
    fn a_citation_with_no_affixes_is_untouched() {
        let rendered = "(Ginzburg, 1976, p. 45)";

        assert_eq!(apply_affixes(rendered, "", ""), rendered);
        assert_eq!(apply_affixes(rendered, "   ", "  "), rendered);
    }

    /// CSL puts affixes inside the brackets. Outside is where a naive
    /// concatenation puts them, and it reads as a different sentence.
    #[test]
    fn a_bracket_style_takes_its_affixes_inside_the_brackets() {
        let out = apply_affixes("(Ginzburg, 1976, p. 45)", "see", "and passim");

        assert_eq!(out, "(see Ginzburg, 1976, p. 45, and passim)");
    }

    #[test]
    fn a_bracket_style_takes_a_prefix_alone() {
        assert_eq!(
            apply_affixes("(Ginzburg 1976, 45)", "see", ""),
            "(see Ginzburg 1976, 45)"
        );
    }

    #[test]
    fn a_bracket_style_takes_a_suffix_alone() {
        assert_eq!(
            apply_affixes("(Ginzburg 1976, 45)", "", "and passim"),
            "(Ginzburg 1976, 45, and passim)"
        );
    }

    /// The style's own brackets are kept rather than rebuilt: a numeric style
    /// uses square ones and normalising them would change the style.
    #[test]
    fn the_styles_own_brackets_survive() {
        assert_eq!(apply_affixes("[1]", "see", ""), "[see 1]");
    }

    /// S3 recorded `45., and passim` as a defect of this wrapper. A note style
    /// ends in its own period, and a suffix after it reads as a new sentence.
    #[test]
    fn a_note_style_loses_its_terminal_period_before_a_suffix() {
        let rendered = "Carlo Ginzburg, Il formaggio e i vermi (Einaudi, 1976), 45.";

        let out = apply_affixes(rendered, "see", "and passim");

        assert_eq!(
            out,
            "see Carlo Ginzburg, Il formaggio e i vermi (Einaudi, 1976), 45, and passim"
        );
        assert!(
            !out.contains("., and"),
            "the terminal period survived: {out}"
        );
    }

    /// Without a suffix there is nothing to make room for, so the style's own
    /// punctuation stays exactly as it wrote it.
    #[test]
    fn a_note_style_keeps_its_period_when_only_a_prefix_is_added() {
        let rendered = "Carlo Ginzburg, Il formaggio e i vermi (Einaudi, 1976), 45.";

        assert_eq!(
            apply_affixes(rendered, "see", ""),
            "see Carlo Ginzburg, Il formaggio e i vermi (Einaudi, 1976), 45."
        );
    }

    /// The shape is read from what the style produced, not from its name, so a
    /// style this build has never seen is still handled correctly.
    #[test]
    fn the_shape_is_recognised_from_the_rendering_itself() {
        assert_eq!(shape_of("(Ginzburg, 1976)"), StyleShape::Bracketed);
        assert_eq!(shape_of("[1]"), StyleShape::Bracketed);
        assert_eq!(shape_of("Carlo Ginzburg, … 45."), StyleShape::Note);
        assert_eq!(shape_of("Ginzburg (1976)"), StyleShape::Note);
    }

    /// `Ginzburg (1976)` is a prose citation, not a bracketed one: the brackets
    /// are around part of it, not around the whole. Treating it as bracketed
    /// would put the prefix in the middle of the citation.
    #[test]
    fn a_prose_citation_is_not_mistaken_for_a_bracketed_one() {
        assert_eq!(
            apply_affixes("Ginzburg (1976)", "see", ""),
            "see Ginzburg (1976)"
        );
    }
}
