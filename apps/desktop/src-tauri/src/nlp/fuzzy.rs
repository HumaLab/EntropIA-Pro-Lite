//! Approximate term matching for the lexical legs of retrieval.
//!
//! The Rust twin of `packages/store/src/fuzzy.ts`, which holds the reasoning
//! in full. In short: variants come from the index's own vocabulary
//! (`fts_items_vocab`, migration 0037) and are kept only when their document
//! frequency marks them as a misreading of the term (far rarer), the spelling a
//! typo meant (far more common), or — when both are rare — another spelling of
//! a rare name. The two implementations must agree; a test below reads the
//! TypeScript constants and fails if they drift.

use std::collections::HashMap;

use rusqlite::Connection;

/// A variant must differ in document frequency by at least this factor.
const WEIGHT_RATIO: i64 = 4;
/// Misreadings one term may add.
const MAX_MISREADINGS: usize = 8;
/// Below this many documents a count says nothing about spelling.
const RARE_DOCS: i64 = 10;

/// The app setting that turns approximate search off ('off'). Shared with the
/// TypeScript side (`SETTINGS_KEYS.SEARCH_FUZZY`).
pub const SEARCH_FUZZY_SETTING: &str = "search_fuzzy";

const VOCAB_SQL: &str = "SELECT term, doc FROM fts_items_vocab \
     WHERE length(term) >= 4 AND term NOT GLOB '*[0-9]*'";

/// Folds case and the diacritics of Latin letters, as the index tokenizer
/// (`unicode61 remove_diacritics 1`) does. A table rather than a Unicode
/// normalization crate: the corpus is Spanish, and this covers every letter it
/// writes without adding a dependency.
pub fn normalize_term(term: &str) -> String {
    term.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            'ý' | 'ÿ' => 'y',
            other => other,
        })
        .collect()
}

/// Edits a term tolerates: none under 5 letters or with a digit, one up to 7
/// letters, two from 8.
pub fn max_edits_for(term: &str) -> usize {
    let length = term.chars().count();
    if term.chars().any(|c| c.is_ascii_digit()) || length < 5 {
        0
    } else if length < 8 {
        1
    } else {
        2
    }
}

/// Optimal-string-alignment distance, giving up at `max + 1`.
pub fn edit_distance(a: &str, b: &str, max: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max {
        return max + 1;
    }
    let mut before: Option<Vec<usize>> = None;
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut current = vec![i; b.len() + 1];
        let mut row_min = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut value = (previous[j] + 1)
                .min(current[j - 1] + 1)
                .min(previous[j - 1] + cost);
            if let Some(before) = &before {
                if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                    value = value.min(before[j - 2] + 1);
                }
            }
            current[j] = value;
            row_min = row_min.min(value);
        }
        if row_min > max {
            return max + 1;
        }
        before = Some(previous);
        previous = current;
    }
    previous[b.len()]
}

struct Candidate<'a> {
    term: &'a str,
    distance: usize,
    docs: i64,
}

fn by_closeness(x: &Candidate, y: &Candidate) -> std::cmp::Ordering {
    x.distance
        .cmp(&y.distance)
        .then(y.docs.cmp(&x.docs))
        .then(x.term.cmp(y.term))
}

/// The vocabulary terms worth searching alongside `term`.
pub fn pick_variants(term: &str, vocab: &HashMap<String, i64>) -> Vec<String> {
    let needle = normalize_term(term);
    let max = max_edits_for(&needle);
    if max == 0 {
        return Vec::new();
    }
    let needle_len = needle.chars().count();
    let weight = vocab.get(&needle).copied().unwrap_or(0);
    let mut misreadings = Vec::new();
    let mut corrections = Vec::new();

    for (candidate, &docs) in vocab {
        if *candidate == needle || candidate.chars().count().abs_diff(needle_len) > max {
            continue;
        }
        let distance = edit_distance(&needle, candidate, max);
        if distance > max {
            continue;
        }
        let entry = Candidate {
            term: candidate,
            distance,
            docs,
        };
        if docs * WEIGHT_RATIO <= weight {
            misreadings.push(entry);
        } else if docs >= 2.max(weight * WEIGHT_RATIO) {
            corrections.push(entry);
        } else if weight > 0 && weight < RARE_DOCS && docs < RARE_DOCS {
            misreadings.push(entry);
        }
    }

    corrections.sort_by(by_closeness);
    misreadings.sort_by(by_closeness);
    corrections
        .into_iter()
        .take(1)
        .chain(misreadings.into_iter().take(MAX_MISREADINGS))
        .map(|candidate| candidate.term.to_string())
        .collect()
}

/// Whether approximate search is on. Anything but 'off' — including a setting
/// that cannot be read — counts as on.
pub fn fuzzy_enabled(conn: &Connection) -> bool {
    crate::settings::get_setting(conn, SEARCH_FUZZY_SETTING).as_deref() != Some("off")
}

/// An FTS5 expression matching any close variant of `terms` and none of the
/// terms themselves, or `None` when approximate search is off, the vocabulary
/// is unavailable, or no term has a variant.
///
/// Variants only, on purpose: the caller has already run the exact search,
/// and a variant — rare by nature — would outrank the real word under BM25 if
/// both went into one query. Asked separately, its finds can be placed after.
pub fn variants_only_match(conn: &Connection, terms: &[String]) -> Option<String> {
    if terms.is_empty() || !fuzzy_enabled(conn) {
        return None;
    }
    let vocab: HashMap<String, i64> = conn
        .prepare(VOCAB_SQL)
        .ok()?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .ok()?
        .collect::<Result<_, _>>()
        .ok()?;

    // Split the way the index tokenizer does: a query token like `sindicato?`
    // or `¿qué` is one or more vocabulary words with punctuation around them,
    // and looked up whole it would read as a typo of the word it contains.
    let own: Vec<String> = terms
        .iter()
        .flat_map(|term| {
            normalize_term(term)
                .split(|c: char| !c.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect();
    let mut variants: Vec<String> = Vec::new();
    for term in &own {
        for variant in pick_variants(term, &vocab) {
            if !own.contains(&variant) && !variants.contains(&variant) {
                variants.push(variant);
            }
        }
    }
    if variants.is_empty() {
        return None;
    }
    Some(
        variants
            .iter()
            .map(|variant| format!("\"{variant}\""))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab(entries: &[(&str, i64)]) -> HashMap<String, i64> {
        entries.iter().map(|(t, d)| (t.to_string(), *d)).collect()
    }

    #[test]
    fn folds_case_and_accents() {
        assert_eq!(normalize_term("ZÁRATE"), "zarate");
        assert_eq!(normalize_term("Peñón"), "penon");
    }

    #[test]
    fn counts_a_transposition_as_one_edit() {
        assert_eq!(edit_distance("trabajadores", "trabaajdores", 2), 1);
        assert_eq!(edit_distance("sindicato", "gobernador", 2), 3);
    }

    #[test]
    fn allows_edits_by_length_and_never_on_digits() {
        assert_eq!(max_edits_for("otiz"), 0);
        assert_eq!(max_edits_for("ortiz"), 1);
        assert_eq!(max_edits_for("sindicato"), 2);
        assert_eq!(max_edits_for("fs12345"), 0);
    }

    // The same cases, with the same real-corpus counts, as fuzzy.test.ts.
    #[test]
    fn picks_the_same_variants_as_the_typescript_side() {
        let words = vocab(&[
            ("sindicato", 269),
            ("sindigato", 3),
            ("sinicato", 2),
            ("sindicatos", 119),
            ("trabajadores", 284),
            ("trabajdores", 1),
            ("obrero", 240),
            ("obreros", 266),
            ("obreo", 1),
        ]);
        assert_eq!(
            pick_variants("sindicato", &words),
            ["sindigato", "sinicato"]
        );
        assert_eq!(pick_variants("obrero", &words), ["obreo"]);
        assert_eq!(pick_variants("trabajdores", &words), ["trabajadores"]);
        assert_eq!(pick_variants("sindicto", &words), ["sindicato"]);

        let names = vocab(&[
            ("crocitto", 9),
            ("crocito", 3),
            ("crosito", 1),
            ("crositto", 1),
        ]);
        assert_eq!(
            pick_variants("Crositto", &names),
            ["crocitto", "crosito", "crocito"]
        );
        assert_eq!(pick_variants("Crocito", &names), ["crocitto", "crosito"]);
        assert_eq!(pick_variants("Crosito", &names), ["crocito", "crositto"]);
        assert_eq!(pick_variants("crocitoo", &names), ["crocitto"]);
    }

    /// Two implementations of one rule drift silently unless something reads
    /// both. This reads the TypeScript constants and thresholds.
    #[test]
    fn the_rules_match_fuzzy_ts() {
        let ts = include_str!("../../../../../packages/store/src/fuzzy.ts");
        let constant = |name: &str| -> i64 {
            let line = ts
                .lines()
                .find(|line| line.starts_with(&format!("const {name} = ")))
                .unwrap_or_else(|| panic!("fuzzy.ts no longer declares {name}"));
            line.rsplit(' ').next().unwrap().parse().unwrap()
        };
        assert_eq!(constant("WEIGHT_RATIO"), WEIGHT_RATIO);
        assert_eq!(constant("MAX_MISREADINGS") as usize, MAX_MISREADINGS);
        assert_eq!(constant("RARE_DOCS"), RARE_DOCS);
        assert!(ts.contains("if (term.length < 5) return 0"));
        assert!(ts.contains("if (term.length < 8) return 1"));
        // And both read the vocabulary through the same filter.
        let repo = include_str!("../../../../../packages/store/src/repos/fts.repo.ts");
        let squash = |sql: &str| sql.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            squash(repo).contains(&squash(VOCAB_SQL)),
            "fts.repo.ts no longer reads the vocabulary as VOCAB_SQL does"
        );
    }
}
