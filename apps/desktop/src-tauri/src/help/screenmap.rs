//! The screen map the help assistant answers from (S11, D8).
//!
//! `manual/manual-usuario/mapa-de-pantallas.md` is generated from the app's
//! Spanish UI strings by `apps/desktop/scripts/build-help-screen-map.mjs`
//! (`pnpm help:screen-map`) and committed: every screen, tab, button and
//! error message by its real Spanish name. Like the manual it travels
//! compiled into the binary (D1), and like the FAQ it is indexed per `###`
//! section (chapter = `##` area), so the content hash covers it (S9).

use std::sync::OnceLock;

use super::index::{KnowledgeDoc, Source};

/// The generated screen map, next to the user manual.
const SCREEN_MAP: &str = include_str!("../../../../../manual/manual-usuario/mapa-de-pantallas.md");

/// One `###` sub-section of the screen map: the `##` area it belongs to, its
/// own heading, and the `- «Etiqueta»` lines under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScreenMapSection {
    /// The `##` area heading (e.g. `Configuración`, `Pantallas`).
    pub area: String,
    /// The `###` sub-area heading (e.g. `Copias`, `Vistas de la app`).
    pub title: String,
    /// The label lines under the heading.
    pub body: String,
}

/// The generated screen map, parsed once.
pub(crate) fn sections() -> &'static [ScreenMapSection] {
    static SECTIONS: OnceLock<Vec<ScreenMapSection>> = OnceLock::new();
    SECTIONS.get_or_init(|| parse_screen_map(SCREEN_MAP))
}

/// Splits the map at its `##` areas and `###` sub-sections. The `#` title and
/// the `>` generated-file header carry no labels and are dropped, as are
/// sections with no lines.
pub(crate) fn parse_screen_map(text: &str) -> Vec<ScreenMapSection> {
    let mut parsed = Vec::new();
    let mut area = String::new();
    let mut current: Option<(String, Vec<&str>)> = None;

    let mut flush = |current: &mut Option<(String, Vec<&str>)>, area: &str| {
        if let Some((title, lines)) = current.take() {
            let body = lines.join("\n");
            if !area.is_empty() && !body.trim().is_empty() {
                parsed.push(ScreenMapSection {
                    area: area.to_string(),
                    title,
                    body,
                });
            }
        }
    };

    for line in text.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            flush(&mut current, &area);
            area = title.trim().to_string();
        } else if let Some(title) = line.strip_prefix("### ") {
            flush(&mut current, &area);
            current = Some((title.trim().to_string(), Vec::new()));
        } else if line.starts_with("# ") || line.starts_with('>') || line.trim().is_empty() {
            continue;
        } else if let Some((_, lines)) = current.as_mut() {
            lines.push(line);
        }
    }
    flush(&mut current, &area);
    parsed
}

/// The screen map sections as index documents (S9, S11): one doc per `###`
/// section, chapter = `##` area, so the content hash covers the map and each
/// excerpt arrives with the area it belongs to.
pub(crate) fn screenmap_docs() -> Vec<KnowledgeDoc> {
    sections()
        .iter()
        .map(|section| KnowledgeDoc {
            source: Source::ScreenMap,
            chapter: section.area.clone(),
            title: section.title.clone(),
            body: section.body.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_areas_and_subsections() {
        let sample = "# Mapa\n\n> generado, no editar\n\n## Pantallas\n### Vistas de la app\n- «Ayuda» `help`\n### Pestañas\n- «Copias» `settings.backupTab`\n";
        let parsed = parse_screen_map(sample);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].area, "Pantallas");
        assert_eq!(parsed[0].title, "Vistas de la app");
        assert_eq!(parsed[0].body, "- «Ayuda» `help`");
        assert_eq!(parsed[1].title, "Pestañas");
    }

    #[test]
    fn bundled_map_has_many_small_labeled_sections() {
        let all = sections();
        assert!(all.len() > 50, "only {} sections parsed", all.len());
        for section in all {
            assert!(!section.area.is_empty(), "{section:?}");
            assert!(!section.title.is_empty(), "{section:?}");
            assert!(!section.body.is_empty(), "{section:?}");
            assert!(
                section.body.chars().count() < 3000,
                "### {} too big to index: {} chars",
                section.title,
                section.body.chars().count()
            );
        }
    }

    #[test]
    fn bundled_map_names_real_ui_labels() {
        let all = sections();
        assert!(
            all.iter().any(|s| s.body.contains("«Guardar cambios»")),
            "«Guardar cambios» missing"
        );
        assert!(
            all.iter().any(|s| s.body.contains("«Ayuda»")),
            "«Ayuda» missing"
        );
        let screens = all
            .iter()
            .find(|s| s.title == "Vistas de la app")
            .expect("«Vistas de la app» missing");
        assert_eq!(screens.area, "Pantallas");
        assert!(screens.body.contains("`settings`") || screens.body.contains("`help`"));
        let tabs = all
            .iter()
            .find(|s| s.title == "Pestañas de Configuración")
            .expect("settings tabs missing");
        assert!(tabs.body.contains("«Copias»"));
    }
}
