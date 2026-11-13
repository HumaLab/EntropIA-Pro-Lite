//! The user manual the help assistant answers from (S4).
//!
//! The manual is compiled into the binary (D1): every variant and platform
//! carries the manual of its own commit, and no installer resource list has to
//! remember it. Per question only a few relevant sections leave the machine
//! (D2), chosen locally by word overlap, never the whole manual.

use std::sync::OnceLock;

use super::text::fold;

/// The user manual, as published on GitHub Pages.
const MANUAL: &str = include_str!("../../../../../manual/manual-usuario/manual-usuario.md");

/// Marker of the troubleshooting chapter («¿Qué hago si…?»), matched on the
/// title rather than its number so a renumbered manual still finds it.
/// Shared with the index, which pins this chapter on failures.
pub(crate) const TROUBLESHOOTING_MARKER: &str = "¿qué hago si";

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

/// True for the troubleshooting chapter («¿Qué hago si…?»), folding both
/// sides so the match never depends on the manual's accents.
pub(crate) fn is_troubleshooting_title(title: &str) -> bool {
    fold(title).contains(&fold(TROUBLESHOOTING_MARKER))
}

/// The bundled manual, parsed once.
pub(crate) fn sections() -> &'static [ManualSection] {
    static SECTIONS: OnceLock<Vec<ManualSection>> = OnceLock::new();
    SECTIONS.get_or_init(|| parse_sections(MANUAL))
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

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
        assert!(is_troubleshooting_title(&parsed[4].title));
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
            .find(|s| is_troubleshooting_title(&s.title))
            .expect("chapter «¿Qué hago si…?» missing");
        assert!(troubleshooting.body.contains("OpenRouter"));
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
