//! Deterministic local answers from the bundled FAQ (S10, D7).
//!
//! `manual/manual-usuario/preguntas-frecuentes.md` holds the «Primeros pasos»
//! group and the 20 common questions, drafted from the manual for the owner's
//! review. A question that matches an entry above a fixed confidence threshold
//! is answered locally with that entry's text, verbatim, without a key and
//! without network. Questions with diagnostics attached never get a local
//! answer: they need the model.

use std::sync::OnceLock;

use super::{
    index::{KnowledgeDoc, Source},
    text,
};

/// The bundled FAQ, as published next to the user manual.
const FAQ: &str = include_str!("../../../../../manual/manual-usuario/preguntas-frecuentes.md");

/// Group of the deterministic first-steps answers (S10). Test-only: the
/// frontend filters starter buttons by this name on the wire.
#[cfg(test)]
pub(crate) const FIRST_STEPS_GROUP: &str = "Primeros pasos";
/// Group of the 20 common questions (S10). Test-only: counted, not read in
/// production.
#[cfg(test)]
pub(crate) const COMMON_QUESTIONS_GROUP: &str = "Preguntas frecuentes";

/// One FAQ entry: the group it belongs to, the question, its trigger
/// variants, the manual heading it was taken from, and the verbatim answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FaqEntry {
    pub group: String,
    pub question: String,
    pub variants: Vec<String>,
    pub source_chapter: String,
    pub source_title: String,
    pub answer: String,
}

/// The local answer for a question: the matched entry's question (identity),
/// its verbatim text, and the manual heading it comes from (becomes the
/// answer's single source).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalAnswer {
    pub question: String,
    pub answer: String,
    pub chapter: String,
    pub title: String,
}

/// Minimum share of the query's content words the best entry must cover.
/// Tuned with the tests below: full questions score 1.0, the two required
/// misspellings score 1.0 and 0.67, the unrelated question scores 0.
const MIN_COVERAGE: f64 = 0.6;
/// Minimum lead of the best entry over the runner-up. Below it the question
/// is ambiguous (e.g. bare «ocr» matches both the how-to and the failure
/// entries) and falls through to the model.
const MIN_MARGIN: f64 = 0.15;

/// The bundled FAQ, parsed once.
pub(crate) fn entries() -> &'static [FaqEntry] {
    static ENTRIES: OnceLock<Vec<FaqEntry>> = OnceLock::new();
    ENTRIES.get_or_init(|| parse_faq(FAQ))
}

/// Splits the FAQ file at its `##` groups and `###` questions. Each entry
/// carries a `Variantes:` line (`;`-separated triggers), a `Fuente:` line
/// (`chapter › section`, both exact manual headings) and the numbered steps.
pub(crate) fn parse_faq(text: &str) -> Vec<FaqEntry> {
    let mut parsed = Vec::new();
    let mut group = String::new();
    let mut current: Option<FaqEntry> = None;
    let mut answer: Vec<String> = Vec::new();

    for line in text.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            flush(&mut current, &mut answer, &mut parsed);
            group = title.trim().to_string();
        } else if let Some(question) = line.strip_prefix("### ") {
            flush(&mut current, &mut answer, &mut parsed);
            current = Some(FaqEntry {
                group: group.clone(),
                question: question.trim().to_string(),
                variants: Vec::new(),
                source_chapter: String::new(),
                source_title: String::new(),
                answer: String::new(),
            });
        } else if let Some(rest) = line.strip_prefix("Variantes:") {
            if let Some(entry) = current.as_mut() {
                entry.variants = rest
                    .split(';')
                    .map(|variant| variant.trim().to_string())
                    .filter(|variant| !variant.is_empty())
                    .collect();
            }
        } else if let Some(rest) = line.strip_prefix("Fuente:") {
            if let Some(entry) = current.as_mut() {
                let rest = rest.trim();
                match rest.split_once('›') {
                    Some((chapter, title)) => {
                        entry.source_chapter = chapter.trim().to_string();
                        entry.source_title = title.trim().to_string();
                    }
                    None => {
                        entry.source_chapter = rest.to_string();
                        entry.source_title = rest.to_string();
                    }
                }
            }
        } else if line.starts_with("# ") {
            flush(&mut current, &mut answer, &mut parsed);
        } else if current.is_some() {
            answer.push(line.to_string());
        }
    }
    flush(&mut current, &mut answer, &mut parsed);
    parsed
}

fn flush(current: &mut Option<FaqEntry>, answer: &mut Vec<String>, parsed: &mut Vec<FaqEntry>) {
    if let Some(mut entry) = current.take() {
        while answer.last().is_some_and(|line| line.trim().is_empty()) {
            answer.pop();
        }
        entry.answer = answer.join("\n");
        if !entry.question.is_empty() {
            parsed.push(entry);
        }
    }
    answer.clear();
}

/// The FAQ entries as index documents (S9): the question plus its variants as
/// the title field, so the content hash covers them and the index retrieves
/// them like any other knowledge.
pub(crate) fn faq_docs() -> Vec<KnowledgeDoc> {
    entries()
        .iter()
        .map(|entry| {
            let mut title = entry.question.clone();
            for variant in &entry.variants {
                title.push_str("; ");
                title.push_str(variant);
            }
            KnowledgeDoc {
                source: Source::Faq,
                chapter: entry.group.clone(),
                title,
                body: entry.answer.clone(),
            }
        })
        .collect()
}

/// The deterministic local answer for `question`, or `None` when it should go
/// to the model. Questions with diagnostics attached never get a local answer
/// (they need the model); without diagnostics the best entry answers when it
/// covers most of the query's content words and beats the runner-up by a
/// margin. Pure and synchronous: unit-testable with no key and no network.
pub(crate) fn answer_locally(diagnostics: Option<&str>, question: &str) -> Option<LocalAnswer> {
    if diagnostics.is_some_and(|report| !report.trim().is_empty()) {
        return None;
    }
    find_local_answer(question)
}

/// The best FAQ entry for `question` under the coverage rule, if any.
pub(crate) fn find_local_answer(question: &str) -> Option<LocalAnswer> {
    let query = content_words(question);
    if query.is_empty() {
        return None;
    }
    let mut scored: Vec<(f64, usize, &FaqEntry)> = entries()
        .iter()
        .map(|entry| {
            let (matched, total) = coverage(entry, &query);
            let ratio = if total == 0 {
                0.0
            } else {
                matched as f64 / total as f64
            };
            (ratio, matched, entry)
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.question.cmp(&b.2.question))
    });
    let (best_ratio, _, best) = scored.first()?;
    let second_ratio = scored.get(1).map(|scored| scored.0).unwrap_or(0.0);
    if *best_ratio >= MIN_COVERAGE && *best_ratio - second_ratio >= MIN_MARGIN {
        Some(LocalAnswer {
            question: best.question.clone(),
            answer: best.answer.clone(),
            chapter: best.source_chapter.clone(),
            title: best.source_title.clone(),
        })
    } else {
        None
    }
}

/// The query's content words: accent-folded, alphanumeric, >= 3 chars,
/// stopwords dropped, deduplicated. Covered by the confidence rule.
pub(crate) fn content_words(question: &str) -> Vec<String> {
    let mut words = Vec::new();
    for word in text::fold(question).split(|c: char| !c.is_alphanumeric()) {
        if word.chars().count() < 3 || text::STOPWORDS.contains(&word) {
            continue;
        }
        let owned = word.to_string();
        if !words.contains(&owned) {
            words.push(owned);
        }
    }
    words
}

/// True when two folded words count as the same for coverage: identical, the
/// same 5-letter stem (covers transcribo/transcripción), one a prefix of the
/// other (covers lote/lotes), or a misspelled word that keeps its start
/// (impotar/importar: first trigram plus two more shared).
pub(crate) fn words_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (alen, blen) = (a.chars().count(), b.chars().count());
    if alen >= 5 && blen >= 5 {
        let a5: String = a.chars().take(5).collect();
        let b5: String = b.chars().take(5).collect();
        if a5 == b5 {
            return true;
        }
    }
    // Plural and diminutive forms (lote/lotes): one word extends the other.
    // Kept strict (not a 3-letter root) so descargar stays away from
    // deshacer; verb forms like creo/crear meet through the exact variants
    // each entry carries instead.
    if alen >= 4 && blen >= 4 && (a.starts_with(b) || b.starts_with(a)) {
        return true;
    }
    let a_trigrams = word_trigrams(a);
    if a_trigrams.is_empty() {
        return false;
    }
    let b_set: std::collections::HashSet<String> = word_trigrams(b).into_iter().collect();
    // A single misspelled word keeps its start (impotar/importar share
    // «imp» plus two more): require the first trigram plus two more, so
    // middle-only lookalikes (exportar/importar) and short overlaps
    // (escaneo/descargar) stay apart.
    let first: String = a.chars().take(3).collect();
    b_set.contains(&first) && a_trigrams.iter().filter(|t| b_set.contains(*t)).count() >= 3
}

fn word_trigrams(word: &str) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    if chars.len() < 3 {
        return Vec::new();
    }
    chars
        .windows(3)
        .map(|window| window.iter().collect())
        .collect()
}

/// How many of the query words `entry` (question + variants) covers.
pub(crate) fn coverage(entry: &FaqEntry, query: &[String]) -> (usize, usize) {
    let mut text = entry.question.clone();
    for variant in &entry.variants {
        text.push(' ');
        text.push_str(variant);
    }
    let entry_words = content_words(&text);
    let matched = query
        .iter()
        .filter(|query_word| {
            entry_words
                .iter()
                .any(|entry_word| words_match(query_word, entry_word))
        })
        .count();
    (matched, query.len())
}

#[cfg(test)]
mod tests {
    use super::super::manual;
    use super::*;

    fn primeros() -> Vec<FaqEntry> {
        entries()
            .iter()
            .filter(|entry| entry.group == FIRST_STEPS_GROUP)
            .cloned()
            .collect()
    }

    #[test]
    fn groups_and_counts() {
        let groups: Vec<&str> = entries()
            .iter()
            .map(|entry| entry.group.as_str())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        assert!(groups.contains(&FIRST_STEPS_GROUP), "{groups:?}");
        assert!(groups.contains(&COMMON_QUESTIONS_GROUP), "{groups:?}");
        assert_eq!(
            primeros().len(),
            10,
            "«Primeros pasos» holds about 10 entries"
        );
        let common = entries()
            .iter()
            .filter(|entry| entry.group == COMMON_QUESTIONS_GROUP)
            .count();
        assert_eq!(common, 20, "exactly 20 common questions");
    }

    #[test]
    fn every_entry_has_variants_source_and_answer() {
        for entry in entries() {
            assert!(!entry.question.is_empty(), "{entry:?}");
            assert_eq!(
                entry.variants.len(),
                3,
                "{:?} must carry 3 trigger variants",
                entry.question
            );
            assert!(
                entry
                    .variants
                    .iter()
                    .all(|variant| !variant.trim().is_empty()),
                "{:?}",
                entry.question
            );
            assert!(!entry.source_chapter.is_empty(), "{:?}", entry.question);
            assert!(!entry.source_title.is_empty(), "{:?}", entry.question);
            assert!(
                entry.answer.contains('1'),
                "{:?} must hold numbered steps",
                entry.question
            );
        }
    }

    #[test]
    fn every_fuente_heading_exists_in_the_manual() {
        use std::collections::HashSet;
        let sections = manual::sections();
        let titles: HashSet<&str> = sections.iter().map(|s| s.title.as_str()).collect();
        let chapters: HashSet<&str> = sections.iter().map(|s| s.chapter.as_str()).collect();
        for entry in entries() {
            assert!(
                titles.contains(entry.source_title.as_str())
                    || chapters.contains(entry.source_title.as_str()),
                "{:?}: title {:?} is not a manual heading",
                entry.question,
                entry.source_title
            );
            assert!(
                titles.contains(entry.source_chapter.as_str())
                    || chapters.contains(entry.source_chapter.as_str()),
                "{:?}: chapter {:?} is not a manual heading",
                entry.question,
                entry.source_chapter
            );
        }
    }

    #[test]
    fn each_primeros_pasos_question_matches_locally() {
        for entry in primeros() {
            let matched = find_local_answer(&entry.question);
            assert_eq!(
                matched.as_ref().map(|answer| answer.question.as_str()),
                Some(entry.question.as_str()),
                "{:?} must answer locally",
                entry.question
            );
            assert_eq!(
                matched.as_ref().map(|answer| answer.answer.as_str()),
                Some(entry.answer.as_str()),
                "{:?} must answer verbatim",
                entry.question
            );
        }
    }

    #[test]
    fn each_primeros_pasos_variant_matches_its_entry() {
        for entry in primeros() {
            for variant in &entry.variants {
                let matched = find_local_answer(variant);
                assert_eq!(
                    matched.as_ref().map(|answer| answer.question.as_str()),
                    Some(entry.question.as_str()),
                    "variant {variant:?} must reach {:?}",
                    entry.question
                );
            }
        }
    }

    #[test]
    fn misspelled_questions_still_match_locally() {
        let matched = find_local_answer("como impotar un pdf").expect("must match");
        assert_eq!(matched.question, "¿Cómo importo un PDF?");
        let matched = find_local_answer("cmo creo una coleccion").expect("must match");
        assert_eq!(matched.question, "¿Cómo creo mi primera colección?");
    }

    #[test]
    fn an_assemblyai_failure_is_not_answered_by_the_generic_audio_entry() {
        let matched = find_local_answer("falló mi transcripción con error 401 de AssemblyAI")
            .expect("must match");
        assert_ne!(matched.question, "¿Cómo transcribo un audio?");
        assert_eq!(
            matched.question,
            "La primera transcripción con AssemblyAI falla, ¿qué reviso?"
        );
    }

    #[test]
    fn an_unrelated_question_falls_through_to_the_model() {
        assert_eq!(find_local_answer("¿Quién escribió el Facundo?"), None);
    }

    #[test]
    fn a_capabilities_question_falls_through_to_the_agent() {
        // S22: «¿En qué me puedes ayudar?» is not small talk and matches
        // no FAQ entry: it goes to the agent, not to a local answer.
        assert_eq!(find_local_answer("¿En qué me puedes ayudar?"), None);
        assert_eq!(find_local_answer("en que me puedes ayudar"), None);
    }

    #[test]
    fn empty_and_stopword_only_questions_have_no_local_answer() {
        assert_eq!(find_local_answer(""), None);
        assert_eq!(find_local_answer("   "), None);
        assert_eq!(find_local_answer("¿cómo que para con los las?"), None);
    }

    #[test]
    fn questions_with_diagnostics_never_get_a_local_answer() {
        let question = "¿Cómo importo un PDF?";
        assert!(find_local_answer(question).is_some());
        assert_eq!(
            answer_locally(Some("Producto: EntropIA Lite\n- algo"), question),
            None
        );
        assert!(answer_locally(None, question).is_some());
    }

    #[test]
    fn local_answers_need_no_key() {
        // The decision is pure: no app state, no keyring, no network.
        let matched = answer_locally(None, "¿Cómo creo mi primera colección?")
            .expect("must match without a key");
        assert_eq!(matched.question, "¿Cómo creo mi primera colección?");
        assert!(matched.answer.contains("Nueva colección"));
    }
}
