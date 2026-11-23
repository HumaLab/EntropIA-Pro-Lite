//! A short passage around the first word a search matched, for result lists.
//!
//! Words compare the way the index tokenizer (`unicode61 remove_diacritics 1`)
//! does: whole words, case and accents folded. `huelga` finds `HUELGA` and
//! `Huelga,` but not `huelgas`.

use serde::Serialize;

use super::fuzzy::normalize_term;

/// Chars of context kept before the first match.
const BEFORE: usize = 60;
/// Chars kept from the first match on.
const AFTER: usize = 160;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetSegment {
    pub text: String,
    pub is_match: bool,
}

/// Byte ranges of the words in `text`: runs of letters and digits.
fn words(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (index, ch) in text.char_indices() {
        if ch.is_alphanumeric() {
            start.get_or_insert(index);
        } else if let Some(begin) = start.take() {
            out.push((begin, index));
        }
    }
    if let Some(begin) = start {
        out.push((begin, text.len()));
    }
    out
}

/// Collapses every run of whitespace, line breaks included, into one space.
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(ch);
            in_space = false;
        }
    }
    out
}

/// The byte offset `chars` characters before or after `from`.
fn step_back(text: &str, from: usize, chars: usize) -> usize {
    text[..from]
        .char_indices()
        .rev()
        .nth(chars.saturating_sub(1))
        .map_or(0, |(index, _)| index)
}

fn step_forward(text: &str, from: usize, chars: usize) -> usize {
    text[from..]
        .char_indices()
        .nth(chars)
        .map_or(text.len(), |(index, _)| from + index)
}

/// The passage around the first word of `text` that matches one of `terms`,
/// with every matching word in it marked. `None` when no word matches: the
/// search found the item by its title or metadata.
pub fn snippet(text: &str, terms: &[String]) -> Option<Vec<SnippetSegment>> {
    let wanted: Vec<String> = terms
        .iter()
        .flat_map(|term| {
            words(term)
                .into_iter()
                .map(|(s, e)| normalize_term(&term[s..e]))
        })
        .collect();
    if wanted.is_empty() {
        return None;
    }

    let all = words(text);
    let is_match = |&(s, e): &(usize, usize)| wanted.contains(&normalize_term(&text[s..e]));
    let first = *all.iter().find(|word| is_match(word))?;

    // Cut on word boundaries: start at the first word inside the window, end
    // after the last one.
    let lower = step_back(text, first.0, BEFORE);
    let upper = step_forward(text, first.0, AFTER);
    let in_window: Vec<(usize, usize)> = all
        .iter()
        .copied()
        .filter(|&(s, e)| s >= lower && e <= upper)
        .collect();
    let start = in_window.first().map_or(first.0, |w| w.0);
    let last_word_end = in_window.last().map_or(first.1, |w| w.1);
    // Keep the punctuation attached to the last word: `Plata.`, not `Plata`.
    let end = text[last_word_end..]
        .char_indices()
        .find(|&(_, ch)| ch.is_whitespace() || ch.is_alphanumeric())
        .map_or(text.len(), |(index, _)| last_word_end + index);

    let mut segments = Vec::new();
    let mut push = |text: String, is_match: bool| {
        if text.is_empty() {
            return;
        }
        match segments.last_mut() {
            Some(SnippetSegment {
                text: last,
                is_match: m,
            }) if *m == is_match => last.push_str(&text),
            _ => segments.push(SnippetSegment { text, is_match }),
        }
    };

    if !text[..start].trim().is_empty() {
        push("…".to_string(), false);
    }
    let mut cursor = start;
    for word in in_window.iter().filter(|word| is_match(word)) {
        push(collapse_whitespace(&text[cursor..word.0]), false);
        push(text[word.0..word.1].to_string(), true);
        cursor = word.1;
    }
    push(collapse_whitespace(&text[cursor..end]), false);
    if !text[end..].trim().is_empty() {
        push("…".to_string(), false);
    }
    Some(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(list: &[&str]) -> Vec<String> {
        list.iter().map(|t| t.to_string()).collect()
    }

    fn render(segments: &[SnippetSegment]) -> String {
        segments
            .iter()
            .map(|s| {
                if s.is_match {
                    format!("[{}]", s.text)
                } else {
                    s.text.clone()
                }
            })
            .collect()
    }

    #[test]
    fn marks_the_matching_words_folding_case_and_accents() {
        let text = "La ASAMBLEA votó.\nOtra asamblea, en Mar del Plata.";
        let out = snippet(text, &terms(&["asamblea"])).unwrap();
        assert_eq!(
            render(&out),
            "La [ASAMBLEA] votó. Otra [asamblea], en Mar del Plata."
        );

        let out = snippet("Se reunió el sindicato.", &terms(&["reunio"])).unwrap();
        assert_eq!(render(&out), "Se [reunió] el sindicato.");
    }

    #[test]
    fn matches_whole_words_only() {
        assert_eq!(snippet("Las huelgas siguieron.", &terms(&["huelga"])), None);
    }

    #[test]
    fn says_nothing_when_no_word_of_the_text_matches() {
        assert_eq!(
            snippet("Texto sin la palabra.", &terms(&["asamblea"])),
            None
        );
        assert_eq!(snippet("Texto.", &[]), None);
    }

    #[test]
    fn a_long_text_is_cut_on_word_boundaries_around_the_first_match() {
        let before = "palabra ".repeat(30);
        let after = "resto ".repeat(60);
        let text = format!("{before}la huelga empezó {after}");
        let out = render(&snippet(&text, &terms(&["huelga"])).unwrap());

        assert!(out.starts_with("…palabra"), "{out}");
        assert!(out.contains("la [huelga] empezó"), "{out}");
        assert!(out.ends_with("resto…"), "{out}");
        assert!(out.chars().count() < 240, "{out}");
    }

    #[test]
    fn every_term_of_the_query_is_marked() {
        let text = "Paro general y asamblea en la planta.";
        let out = snippet(text, &terms(&["asamblea", "paro"])).unwrap();
        assert_eq!(render(&out), "[Paro] general y [asamblea] en la planta.");
    }
}
