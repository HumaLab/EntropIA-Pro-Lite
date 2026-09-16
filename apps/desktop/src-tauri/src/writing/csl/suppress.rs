//! Citing a work whose author you already named (plan-editor.md §11.5).
//!
//! # What this is for
//!
//! When the author is already in the sentence, repeating them in the citation
//! reads badly:
//!
//! > Como sostiene **Ginzburg**, el molinero leía de otro modo
//! > (**Ginzburg**, 1976, p. 45).
//!
//! CSL calls the fix `suppress-author`; Zotero shows it as a checkbox. What is
//! wanted is everything *except* the author, page included:
//!
//! > Como sostiene **Ginzburg** (1976, p. 45), el molinero leía de otro modo.
//!
//! # Why it is done by subtraction
//!
//! Spike S3 measured that hayagriva has no `suppress-author`. The nearest
//! things it does offer both throw the locator away: `CitePurpose::Prose` gives
//! `Ginzburg (1976)` and `CitePurpose::Year` gives `1976`, neither of which is
//! "everything except the author" — and a citation that silently loses its page
//! number is worse than no feature at all for anyone citing sources properly.
//!
//! But hayagriva *can* render the author alone. So the citation is rendered
//! twice and one is subtracted from the other:
//!
//! ```text
//! (Ginzburg, 1976, p. 45)  −  Ginzburg  →  (1976, p. 45)
//! ```
//!
//! This is the same kind of work as [`super::affix`] and carries the same
//! warning S3 attached to it: **this is our code doing CSL's job**, so it is a
//! tested helper with its cases written down, not a string patch.
//!
//! # What it refuses to do
//!
//! Subtraction only works when the author actually appears in the rendering. If
//! it does not — a style that abbreviates differently, a rendering this build
//! did not anticipate — the answer is `None` rather than a mangled citation.
//! Handing back the full citation as though suppression had happened would make
//! the checkbox look broken; saying it could not be done is the honest answer.

/// Separators a style may leave behind once the author is removed.
const SEPARATORS: [&str; 4] = [", ", "; ", ". ", " "];

/// The citation with the author taken out, or `None` when it cannot be done.
///
/// `None` is a real answer and the caller must handle it: it means the full
/// citation stands and the writer should be told, not that nothing happened.
pub fn without_author(rendered: &str, author: &str) -> Option<String> {
    let author = author.trim();
    if author.is_empty() {
        return None;
    }

    // The first occurrence, because every style that names an author puts them
    // before the year. A later one would be inside a title.
    let at = rendered.find(author)?;
    let after = at + author.len();

    let tail = &rendered[after..];
    // Take the separator the style used between the author and what follows,
    // or the citation keeps a stray comma where the name used to be.
    let consumed = SEPARATORS
        .iter()
        .find(|separator| tail.starts_with(*separator))
        .map_or(0, |separator| separator.len());

    let mut out = String::with_capacity(rendered.len());
    out.push_str(&rendered[..at]);
    out.push_str(&rendered[after + consumed..]);

    // A style that puts the author last leaves its separator dangling before
    // the closing bracket: `(1976, )`.
    let cleaned = tidy(&out);
    // Empty brackets are not an empty string, but they are just as much "no
    // citation left": a rendering with nothing to read has to be refused rather
    // than shown as `()`.
    if !cleaned.chars().any(char::is_alphanumeric) {
        return None;
    }
    Some(cleaned)
}

/// Removes punctuation the author was holding up.
fn tidy(text: &str) -> String {
    let mut out = text.to_string();
    for (open, close) in [('(', ')'), ('[', ']')] {
        let dangling_open = format!("{open}, ");
        out = out.replace(&dangling_open, &open.to_string());
        let dangling_close = format!(", {close}");
        out = out.replace(&dangling_close, &close.to_string());
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The case the feature exists for, in the style most of the corpus uses.
    #[test]
    fn an_author_date_citation_keeps_its_year_and_its_page() {
        let out = without_author("(Ginzburg, 1976, p. 45)", "Ginzburg");

        assert_eq!(out.as_deref(), Some("(1976, p. 45)"));
    }

    /// The whole reason for subtracting rather than using `CitePurpose::Year`:
    /// that one throws the page away, and a citation without its page is not a
    /// citation anyone can check.
    #[test]
    fn the_locator_survives_which_is_the_entire_point() {
        let out = without_author("(Ginzburg, 1976, p. 45)", "Ginzburg").unwrap();

        assert!(out.contains("p. 45"), "the page was lost: {out}");
    }

    #[test]
    fn chicago_author_date_separates_with_a_space_and_still_works() {
        let out = without_author("(Ginzburg 1976, 45)", "Ginzburg");

        assert_eq!(out.as_deref(), Some("(1976, 45)"));
    }

    #[test]
    fn several_authors_are_removed_together() {
        let out = without_author("(Darnton & Ginzburg, 1984)", "Darnton & Ginzburg");

        assert_eq!(out.as_deref(), Some("(1984)"));
    }

    /// `et al.` is part of what the author-only rendering returns, so it goes
    /// with the rest rather than being left stranded before the year.
    #[test]
    fn et_al_goes_with_the_author_it_belongs_to() {
        let out = without_author("(Ginzburg et al., 1976, p. 45)", "Ginzburg et al.");

        assert_eq!(out.as_deref(), Some("(1976, p. 45)"));
    }

    /// A surname that also appears in the title must not be removed from there:
    /// the citation names its author first, so the first occurrence is the one.
    #[test]
    fn a_surname_inside_the_title_is_left_alone() {
        let out = without_author(
            "Carlo Ginzburg, El queso y los gusanos de Ginzburg (Einaudi, 1976), 45.",
            "Carlo Ginzburg",
        );

        assert_eq!(
            out.as_deref(),
            Some("El queso y los gusanos de Ginzburg (Einaudi, 1976), 45.")
        );
    }

    /// The honest refusal. Handing back the full citation as though suppression
    /// had happened would make the checkbox look broken instead of unavailable.
    #[test]
    fn a_rendering_the_author_does_not_appear_in_is_refused_not_mangled() {
        assert_eq!(without_author("(1976, p. 45)", "Ginzburg"), None);
        assert_eq!(without_author("[1]", "Ginzburg"), None);
    }

    #[test]
    fn an_empty_author_is_refused_rather_than_removing_nothing_successfully() {
        assert_eq!(without_author("(Ginzburg, 1976)", ""), None);
        assert_eq!(without_author("(Ginzburg, 1976)", "   "), None);
    }

    /// Removing the author must not leave the punctuation it was holding up.
    #[test]
    fn no_stray_punctuation_is_left_where_the_name_was() {
        let out = without_author("(Ginzburg, 1976)", "Ginzburg").unwrap();

        assert!(!out.contains("(,"), "stray comma: {out}");
        assert!(!out.contains(", )"), "stray comma: {out}");
        assert_eq!(out, "(1976)");
    }

    /// A citation that is nothing but its author has nothing left to say, so it
    /// is refused rather than rendered as an empty bracket.
    #[test]
    fn a_citation_of_only_the_author_is_refused() {
        assert_eq!(without_author("Ginzburg", "Ginzburg"), None);
        assert_eq!(without_author("(Ginzburg)", "Ginzburg"), None);
    }

    /// Accents and non-ASCII names must not be sliced through the middle of a
    /// character, which is a panic in Rust rather than a wrong answer.
    #[test]
    fn a_name_with_accents_is_removed_whole() {
        let out = without_author("(Martínez Sarasola, 1992, p. 87)", "Martínez Sarasola");

        assert_eq!(out.as_deref(), Some("(1992, p. 87)"));
    }
}
