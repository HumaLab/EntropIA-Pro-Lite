//! The user manual the help assistant answers from (S4).
//!
//! The manual is compiled into the binary (D1): every variant and platform
//! carries the manual of its own commit, and no installer resource list has to
//! remember it. Per question only a few relevant sections leave the machine
//! (D2), chosen locally by word overlap, never the whole manual.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// The user manual, as published on GitHub Pages.
const MANUAL: &str = include_str!("../../../../../manual/manual-usuario/manual-usuario.md");

/// Marker of the troubleshooting chapter («¿Qué hago si…?»), matched on the
/// title rather than its number so a renumbered manual still finds it.
const TROUBLESHOOTING_MARKER: &str = "¿qué hago si";

/// One `##` chapter or `###` section of the manual.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ManualSection {
    /// The chapter heading the section belongs to (its own title for a chapter).
    pub chapter: String,
    /// The section heading, without the `#` marks.
    pub title: String,
    /// Text under the heading, without images and HTML anchors.
    pub body: String,
}

impl ManualSection {
    pub(crate) fn is_troubleshooting(&self) -> bool {
        self.title.to_lowercase().contains(TROUBLESHOOTING_MARKER)
    }

    /// Characters the section takes in the prompt.
    pub(crate) fn len_chars(&self) -> usize {
        self.title.chars().count() + self.body.chars().count()
    }
}

/// The bundled manual, parsed once.
pub(crate) fn sections() -> &'static [ManualSection] {
    static SECTIONS: OnceLock<Vec<ManualSection>> = OnceLock::new();
    SECTIONS.get_or_init(|| parse_sections(MANUAL))
}

/// Most sections that go to the model besides the troubleshooting chapter.
const MAX_RANKED_SECTIONS: usize = 4;

/// Splits the manual at its `##` and `###` headings. The `#` title, the index
/// chapters (a list of links, no help) and sections with no text are dropped;
/// images and HTML anchors are removed from the bodies.
pub(crate) fn parse_sections(text: &str) -> Vec<ManualSection> {
    let mut sections = Vec::new();
    let mut chapter = String::new();
    let mut current: Option<(String, Vec<&str>)> = None;
    let mut skipping_chapter = false;

    let mut flush = |current: &mut Option<(String, Vec<&str>)>, chapter: &str, skip: bool| {
        if let Some((title, lines)) = current.take() {
            let body = clean_body(&lines);
            if !skip && !body.is_empty() {
                sections.push(ManualSection {
                    chapter: chapter.to_string(),
                    title,
                    body,
                });
            }
        }
    };

    for line in text.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            flush(&mut current, &chapter, skipping_chapter);
            chapter = title.trim().to_string();
            skipping_chapter = chapter.starts_with("Índice");
            current = Some((chapter.clone(), Vec::new()));
        } else if let Some(title) = line.strip_prefix("### ") {
            flush(&mut current, &chapter, skipping_chapter);
            current = Some((title.trim().to_string(), Vec::new()));
        } else if line.starts_with("# ") {
            flush(&mut current, &chapter, skipping_chapter);
        } else if let Some((_, lines)) = current.as_mut() {
            lines.push(line);
        }
    }
    flush(&mut current, &chapter, skipping_chapter);
    sections
}

fn clean_body(lines: &[&str]) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("![") || trimmed.starts_with("<a id=") {
            continue;
        }
        if trimmed.is_empty() && out.last().is_none_or(|last| last.trim().is_empty()) {
            continue;
        }
        out.push(line.trim_end());
    }
    while out.last().is_some_and(|last| last.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// Lowercases and drops Spanish accents, so «Transcripción» meets «transcripcion».
fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

const STOPWORDS: &[&str] = &[
    "que", "como", "para", "por", "con", "los", "las", "una", "uno", "unos", "unas", "del", "mis",
    "sus", "este", "esta", "esto", "estos", "estas", "ese", "esa", "eso", "hay", "puedo", "quiero",
    "hago", "hacer", "donde", "cuando", "porque", "pero", "sin", "sobre", "entre", "todo", "todos",
    "tiene", "tengo", "cual", "mas", "muy", "ya", "les", "le", "me", "mi", "se", "si", "no", "es",
    "son", "al", "el", "la", "lo", "de", "en", "y", "o", "un", "a", "the", "and", "for", "how",
    "what", "can", "entropia", "app",
];

/// Words of `text` reduced to a five-letter stem, a crude but sufficient
/// match for Spanish inflections («importo», «importar», «importación»).
fn stems(text: &str) -> Vec<String> {
    fold(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 3 && !STOPWORDS.contains(word))
        .map(|word| word.chars().take(5).collect())
        .collect()
}

/// True when the question reads as something going wrong rather than as
/// "how do I", so the troubleshooting chapter goes along.
pub(crate) fn looks_like_failure(question: &str) -> bool {
    const MARKERS: &[&str] = &[
        "error",
        "fall",
        "no funciona",
        "no anda",
        "no puedo",
        "no abre",
        "no aparece",
        "no carga",
        "no me deja",
        "que hago si",
        "problema",
        "se cuelga",
        "se traba",
        "doesn't work",
        "does not work",
        "fail",
    ];
    let folded = fold(question);
    MARKERS.iter().any(|marker| folded.contains(marker))
}

/// The sections that answer `question`, best first, within `budget_chars`.
/// BM25 over stems, with the title counted three times. With
/// `include_troubleshooting` the troubleshooting chapter leads; otherwise it is
/// never picked, so a how-to answer is not padded with unrelated fixes. A
/// section that does not fit the remaining budget is skipped, never cut.
pub(crate) fn select_sections<'a>(
    sections: &'a [ManualSection],
    question: &str,
    include_troubleshooting: bool,
    budget_chars: usize,
) -> Vec<&'a ManualSection> {
    let mut picked: Vec<&ManualSection> = Vec::new();
    let mut used = 0usize;
    let mut take = |section: &'a ManualSection, picked: &mut Vec<&'a ManualSection>| {
        let size = section.len_chars();
        if used + size <= budget_chars {
            used += size;
            picked.push(section);
            true
        } else {
            false
        }
    };

    if include_troubleshooting {
        if let Some(section) = sections.iter().find(|s| s.is_troubleshooting()) {
            take(section, &mut picked);
        }
    }

    let query: HashSet<String> = stems(question).into_iter().collect();
    if query.is_empty() {
        return picked;
    }

    let docs: Vec<(&ManualSection, HashMap<String, f64>, f64)> = sections
        .iter()
        .filter(|s| !s.is_troubleshooting())
        .map(|section| {
            let mut tf: HashMap<String, f64> = HashMap::new();
            let title = stems(&section.title);
            let body = stems(&section.body);
            for stem in &title {
                *tf.entry(stem.clone()).or_default() += 3.0;
            }
            for stem in &body {
                *tf.entry(stem.clone()).or_default() += 1.0;
            }
            let len = (title.len() * 3 + body.len()) as f64;
            (section, tf, len)
        })
        .collect();
    if docs.is_empty() {
        return picked;
    }
    let n = docs.len() as f64;
    let avg_len = (docs.iter().map(|(_, _, len)| len).sum::<f64>() / n).max(1.0);
    let (k1, b) = (1.2, 0.75);

    let mut scored: Vec<(f64, &ManualSection)> = docs
        .iter()
        .map(|(section, tf, len)| {
            let score = query
                .iter()
                .filter_map(|stem| {
                    let freq = *tf.get(stem)?;
                    let df = docs
                        .iter()
                        .filter(|(_, other, _)| other.contains_key(stem))
                        .count() as f64;
                    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                    Some(idf * freq * (k1 + 1.0) / (freq + k1 * (1.0 - b + b * len / avg_len)))
                })
                .sum::<f64>();
            (score, *section)
        })
        .filter(|(score, _)| *score > 0.0)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));

    let mut ranked = 0;
    for (_, section) in scored {
        if ranked == MAX_RANKED_SECTIONS {
            break;
        }
        if take(section, &mut picked) {
            ranked += 1;
        }
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# EntropIA Lite\n\n## Manual de usuario\n\nIntro.\n\n## Índice\n\n- [Capítulo 1](#c1)\n- importar archivos\n\n<a id=\"c1\"></a>\n## Capítulo 1. Inicio rápido\n\nTexto del capítulo.\n\n### 1.1. Importar archivos\n\n![Una imagen](images/x.svg)\n\nArrastrá los PDF a la colección.\n\n### 1.2. Buscar\n\nUsá la barra de búsqueda.\n\n<a id=\"c19\"></a>\n## Capítulo 19. ¿Qué hago si…?\n\n| Problema | Qué revisar |\n|---|---|\n| Falla OpenRouter | Revisá la clave. |\n";

    #[test]
    fn parse_keeps_chapters_and_sections_without_images_anchors_or_index() {
        let parsed = parse_sections(SAMPLE);
        let titles: Vec<&str> = parsed.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(
            titles,
            vec![
                "Manual de usuario",
                "Capítulo 1. Inicio rápido",
                "1.1. Importar archivos",
                "1.2. Buscar",
                "Capítulo 19. ¿Qué hago si…?",
            ]
        );
        let import = &parsed[2];
        assert_eq!(import.chapter, "Capítulo 1. Inicio rápido");
        assert_eq!(import.body, "Arrastrá los PDF a la colección.");
        assert!(parsed.iter().all(|s| !s.body.contains("<a id")));
        assert!(parsed.iter().all(|s| !s.body.contains("![")));
        assert!(parsed[4].is_troubleshooting());
        assert_eq!(parsed[4].chapter, parsed[4].title);
    }

    #[test]
    fn bundled_manual_has_every_chapter_and_the_troubleshooting_table() {
        let all = sections();
        let chapters: HashSet<&str> = all.iter().map(|s| s.chapter.as_str()).collect();
        let numbered = chapters
            .iter()
            .filter(|c| c.starts_with("Capítulo "))
            .count();
        assert!(numbered >= 21, "only {numbered} chapters parsed");
        let troubleshooting = all
            .iter()
            .find(|s| s.is_troubleshooting())
            .expect("chapter «¿Qué hago si…?» missing");
        assert!(troubleshooting.body.contains("OpenRouter"));
    }

    #[test]
    fn a_how_to_question_picks_the_matching_sections_within_budget() {
        let picked = select_sections(
            sections(),
            "¿Cómo importo archivos PDF a una colección?",
            false,
            24_000,
        );
        assert!(!picked.is_empty());
        assert!(picked.len() <= 4, "{} sections", picked.len());
        assert!(
            picked.iter().any(|s| s.title.contains("Importar archivos")),
            "picked: {:?}",
            picked.iter().map(|s| &s.title).collect::<Vec<_>>()
        );
        assert!(picked.iter().all(|s| !s.is_troubleshooting()));
        let total: usize = picked.iter().map(|s| s.len_chars()).sum();
        assert!(total <= 24_000, "{total} chars");
    }

    #[test]
    fn a_failure_puts_the_troubleshooting_chapter_first() {
        let picked = select_sections(
            sections(),
            "falla la transcripción con AssemblyAI",
            true,
            24_000,
        );
        assert!(picked[0].is_troubleshooting());
        assert!(
            picked.len() >= 2,
            "only the troubleshooting chapter was picked"
        );
        assert!(picked.len() <= 5);
        let total: usize = picked.iter().map(|s| s.len_chars()).sum();
        assert!(total <= 24_000, "{total} chars");
    }

    #[test]
    fn a_question_the_manual_does_not_cover_picks_nothing() {
        let parsed = parse_sections(SAMPLE);
        assert!(select_sections(&parsed, "xyzzy plugh", false, 24_000).is_empty());
        let only_help = select_sections(&parsed, "xyzzy plugh", true, 24_000);
        assert_eq!(only_help.len(), 1);
        assert!(only_help[0].is_troubleshooting());
    }

    #[test]
    fn a_section_over_the_budget_is_skipped_not_cut() {
        let parsed = parse_sections(SAMPLE);
        let picked = select_sections(&parsed, "importar archivos", false, 10);
        assert!(picked.is_empty());
    }

    #[test]
    fn failure_wording_is_recognised() {
        for q in [
            "me sale un error al transcribir",
            "no puedo abrir el PDF",
            "la transcripción falló",
            "OpenRouter no funciona",
            "¿Qué hago si no aparece el texto?",
        ] {
            assert!(looks_like_failure(q), "{q}");
        }
        for q in ["¿Cómo creo una colección?", "¿Para qué sirve Escritura?"] {
            assert!(!looks_like_failure(q), "{q}");
        }
    }
}
