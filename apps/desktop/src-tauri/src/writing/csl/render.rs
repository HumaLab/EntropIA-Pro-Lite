//! Rendering citations and the bibliography (plan-editor.md §11.5, §11.6).
//!
//! The engine is hayagriva, reading Zotero's CSL-JSON directly:
//! `citationberg::json::Item` implements `EntryLike`, so there is no conversion
//! layer between what Zotero's local API returns and what gets rendered. Spike
//! S5 confirmed that end-to-end.
//!
//! # What is stored, and what is not
//!
//! §11.5 is explicit: store data equivalent to CSL, never the rendered string.
//! So nothing here caches its own output. A citation's durable form is its
//! item, locator, affixes and suppression flag; the text is derived from those
//! every time, which is what makes switching style re-render everything
//! (criterion 16) instead of leaving old strings behind.
//!
//! # The two things hayagriva does not do
//!
//! Affixes and author suppression are applied after rendering, by
//! [`super::affix`] and [`super::suppress`]. Both carry the warning S3 attached
//! to them — this is our code doing CSL's job — and both have their own tests.

use hayagriva::archive::{locales, ArchivedStyle};
use hayagriva::citationberg::taxonomy::Locator;
use hayagriva::citationberg::{json, Style};
use hayagriva::{
    BibliographyDriver, BibliographyRequest, CitationItem, CitationRequest, LocatorPayload,
    SpecificLocator,
};

/// A coded failure, so the frontend can branch instead of matching prose.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CslError {
    pub code: String,
    pub message: String,
}

impl CslError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

pub type CslResult<T> = Result<T, CslError>;

/// One item of a citation cluster, in its durable form.
///
/// Everything here is CSL-equivalent data. The rendered string is deliberately
/// absent: it is produced from this, never stored beside it.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ClusterItem {
    /// The work, as Zotero's CSL-JSON. Read directly, with no conversion.
    pub csl_json: String,
    /// A page, chapter, section… The kind is named separately because CSL
    /// renders `p. 45` and `Chapter 3` differently.
    pub locator: Option<String>,
    pub locator_kind: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    /// Whether the author is already named in the writer's own sentence.
    #[serde(default)]
    pub suppress_author: bool,
}

/// A rendered cluster, and what could not be honoured about it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RenderedCluster {
    pub text: String,
    /// False when suppression was asked for and could not be done, so the UI
    /// can say so rather than leaving the checkbox looking broken.
    pub author_suppressed: bool,
}

/// The locator kinds worth offering. CSL has more; these are the ones a
/// historian reaches for, and an unknown kind falls back to a page rather than
/// failing the whole citation.
fn locator_of(kind: Option<&str>) -> Locator {
    match kind.unwrap_or("page") {
        "book" => Locator::Book,
        "chapter" => Locator::Chapter,
        "column" => Locator::Column,
        "figure" => Locator::Figure,
        "folio" => Locator::Folio,
        "issue" => Locator::Issue,
        "line" => Locator::Line,
        "note" => Locator::Note,
        "opus" => Locator::Opus,
        "paragraph" => Locator::Paragraph,
        "part" => Locator::Part,
        "section" => Locator::Section,
        "sub-verbo" => Locator::SubVerbo,
        "verse" => Locator::Verse,
        "volume" => Locator::Volume,
        _ => Locator::Page,
    }
}

/// Looks a style up in the bundled archive.
///
/// §11.6 asks for a reduced initial set and a safe path for adding others. The
/// archive is that set — it ships with the binary, so a fresh install renders
/// without downloading anything — and a name it does not know is refused by
/// name rather than silently falling back to APA, which would mis-render a
/// whole manuscript without saying so.
fn style_by_name(name: &str) -> CslResult<hayagriva::citationberg::IndependentStyle> {
    let archived = ArchivedStyle::by_name(name)
        .ok_or_else(|| CslError::new("unknown_style", format!("no bundled style named {name}")))?;
    match archived.get() {
        Style::Independent(style) => Ok(style),
        Style::Dependent(_) => Err(CslError::new(
            "dependent_style",
            format!("{name} is a dependent style and has no rendering of its own"),
        )),
    }
}

fn parse_item(csl_json: &str) -> CslResult<json::Item> {
    serde_json::from_str(csl_json)
        .map_err(|error| CslError::new("invalid_csl_json", format!("{error}")))
}

/// Renders one cluster: several works cited together as one citation (§11.5).
pub fn render_cluster(items: &[ClusterItem], style_name: &str) -> CslResult<RenderedCluster> {
    if items.is_empty() {
        return Err(CslError::new("empty_cluster", "a cluster needs an item"));
    }
    let style = style_by_name(style_name)?;
    let locales = locales();

    let entries: Vec<json::Item> = items
        .iter()
        .map(|item| parse_item(&item.csl_json))
        .collect::<CslResult<_>>()?;

    let citation_items: Vec<CitationItem<json::Item>> = entries
        .iter()
        .zip(items)
        .map(|(entry, input)| {
            let mut cite = CitationItem::with_entry(entry);
            if let Some(locator) = input.locator.as_deref().filter(|l| !l.is_empty()) {
                cite.locator = Some(SpecificLocator(
                    locator_of(input.locator_kind.as_deref()),
                    LocatorPayload::Str(locator),
                ));
            }
            cite
        })
        .collect();

    let mut driver = BibliographyDriver::new();
    driver.citation(CitationRequest::from_items(
        citation_items,
        &style,
        &locales,
    ));
    let rendered = driver.finish(BibliographyRequest {
        style: &style,
        locale: None,
        locale_files: &locales,
    });

    let first = rendered
        .citations
        .first()
        .ok_or_else(|| CslError::new("render_failed", "the engine returned no citation"))?;
    let mut text = first.citation.to_string();
    let mut author_suppressed = false;

    // Suppression before affixes: the prefix belongs in front of what is left,
    // not in front of a name that is about to be removed.
    if items.len() == 1 && items[0].suppress_author {
        if let Some(author) = author_only(&entries[0], &style, &locales) {
            if let Some(without) = super::suppress::without_author(&text, &author) {
                text = without;
                author_suppressed = true;
            }
        }
    }

    let prefix = items[0].prefix.as_deref().unwrap_or("");
    let suffix = items[0].suffix.as_deref().unwrap_or("");
    text = super::affix::apply_affixes(&text, prefix, suffix);

    Ok(RenderedCluster {
        text,
        author_suppressed,
    })
}

/// The author alone, which is the half that gets subtracted.
fn author_only(
    entry: &json::Item,
    style: &hayagriva::citationberg::IndependentStyle,
    locales: &[hayagriva::citationberg::Locale],
) -> Option<String> {
    let mut cite = CitationItem::with_entry(entry);
    cite.purpose = Some(hayagriva::CitePurpose::Author);
    let mut driver = BibliographyDriver::new();
    driver.citation(CitationRequest::from_items(vec![cite], style, locales));
    let rendered = driver.finish(BibliographyRequest {
        style,
        locale: None,
        locale_files: locales,
    });
    let text = rendered.citations.first()?.citation.to_string();
    let trimmed = text.trim().trim_matches(['(', ')', '[', ']']).trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// The bibliography, containing only works that were cited (§11.6, criterion 17).
///
/// A derived view, not a stored list: it is built from the citations handed in,
/// so a work that stops being cited stops appearing without anything having to
/// remember to remove it.
pub fn render_bibliography(cited: &[String], style_name: &str) -> CslResult<Vec<String>> {
    let style = style_by_name(style_name)?;
    let locales = locales();

    let entries: Vec<json::Item> = cited
        .iter()
        .map(|csl_json| parse_item(csl_json))
        .collect::<CslResult<_>>()?;

    let mut driver = BibliographyDriver::new();
    for entry in &entries {
        driver.citation(CitationRequest::from_items(
            vec![CitationItem::with_entry(entry)],
            &style,
            &locales,
        ));
    }
    let rendered = driver.finish(BibliographyRequest {
        style: &style,
        locale: None,
        locale_files: &locales,
    });

    Ok(rendered
        .bibliography
        .map(|bibliography| {
            bibliography
                .items
                .iter()
                .map(|item| item.content.to_string())
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zotero's CSL-JSON, in the shape its local API returns. `language` is
    /// present on purpose: S5 pinned down that Chicago title-cases a work
    /// without one, and hayagriva suppresses that correctly once it is there.
    const GINZBURG: &str = r#"{
        "id": "ginzburg1976",
        "type": "book",
        "title": "Il formaggio e i vermi",
        "author": [{ "family": "Ginzburg", "given": "Carlo" }],
        "issued": { "date-parts": [[1976]] },
        "publisher": "Einaudi",
        "language": "it"
    }"#;

    const DARNTON: &str = r#"{
        "id": "darnton1984",
        "type": "book",
        "title": "The Great Cat Massacre",
        "author": [{ "family": "Darnton", "given": "Robert" }],
        "issued": { "date-parts": [[1984]] },
        "publisher": "Basic Books",
        "language": "en"
    }"#;

    fn item(csl_json: &str) -> ClusterItem {
        ClusterItem {
            csl_json: csl_json.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn renders_a_citation_from_zotero_json_with_no_conversion_layer() {
        let out = render_cluster(&[item(GINZBURG)], "apa").expect("render");

        assert!(out.text.contains("Ginzburg"), "{}", out.text);
        assert!(out.text.contains("1976"), "{}", out.text);
    }

    /// Criterion 15: several works cited together are one citation, not two.
    #[test]
    fn a_cluster_of_two_works_renders_as_one_citation() {
        let out = render_cluster(&[item(GINZBURG), item(DARNTON)], "apa").expect("render");

        assert!(out.text.contains("Ginzburg"), "{}", out.text);
        assert!(out.text.contains("Darnton"), "{}", out.text);
        assert_eq!(out.text.matches('(').count(), 1, "two brackets: {}", out.text);
    }

    #[test]
    fn a_locator_is_rendered_and_its_kind_changes_how() {
        let page = render_cluster(
            &[ClusterItem {
                locator: Some("45".into()),
                ..item(GINZBURG)
            }],
            "apa",
        )
        .expect("page");
        assert!(page.text.contains("45"), "{}", page.text);

        let chapter = render_cluster(
            &[ClusterItem {
                locator: Some("3".into()),
                locator_kind: Some("chapter".into()),
                ..item(GINZBURG)
            }],
            "apa",
        )
        .expect("chapter");
        assert_ne!(page.text, chapter.text, "the locator kind changed nothing");
    }

    /// Criterion 16: changing style re-renders, which is why nothing stores the
    /// rendered string.
    #[test]
    fn switching_style_re_renders_the_same_citation() {
        let apa = render_cluster(&[item(GINZBURG)], "apa").expect("apa");
        let chicago =
            render_cluster(&[item(GINZBURG)], "chicago-author-date").expect("chicago");

        assert_ne!(apa.text, chicago.text);
        assert!(chicago.text.contains("Ginzburg"), "{}", chicago.text);
    }

    /// Criterion 17: a derived view of what was cited, never a stored list.
    #[test]
    fn the_bibliography_holds_only_the_works_that_were_cited() {
        let entries = render_bibliography(&[GINZBURG.to_string()], "apa").expect("bibliography");

        assert_eq!(entries.len(), 1);
        assert!(entries[0].contains("Ginzburg"), "{}", entries[0]);
        assert!(!entries[0].contains("Darnton"), "{}", entries[0]);
    }

    #[test]
    fn a_work_that_stops_being_cited_stops_appearing() {
        let both = render_bibliography(&[GINZBURG.to_string(), DARNTON.to_string()], "apa")
            .expect("both");
        let one = render_bibliography(&[GINZBURG.to_string()], "apa").expect("one");

        assert_eq!(both.len(), 2);
        assert_eq!(one.len(), 1);
    }

    /// The decision the user made: subtraction, because it keeps the page that
    /// hayagriva's own approximations throw away.
    #[test]
    fn suppressing_the_author_keeps_the_locator() {
        let out = render_cluster(
            &[ClusterItem {
                locator: Some("45".into()),
                suppress_author: true,
                ..item(GINZBURG)
            }],
            "apa",
        )
        .expect("render");

        assert!(out.author_suppressed, "suppression was not applied: {}", out.text);
        assert!(!out.text.contains("Ginzburg"), "the author survived: {}", out.text);
        assert!(out.text.contains("1976"), "the year was lost: {}", out.text);
        assert!(out.text.contains("45"), "the page was lost: {}", out.text);
    }

    /// Not asking for it must leave the citation exactly as the style rendered it.
    #[test]
    fn a_citation_that_did_not_ask_for_suppression_keeps_its_author() {
        let out = render_cluster(&[item(GINZBURG)], "apa").expect("render");

        assert!(!out.author_suppressed);
        assert!(out.text.contains("Ginzburg"));
    }

    #[test]
    fn affixes_are_applied_to_the_rendered_citation() {
        let out = render_cluster(
            &[ClusterItem {
                prefix: Some("ver".into()),
                suffix: Some("y ss.".into()),
                ..item(GINZBURG)
            }],
            "apa",
        )
        .expect("render");

        assert!(out.text.contains("ver"), "{}", out.text);
        assert!(out.text.contains("y ss."), "{}", out.text);
    }

    /// Falling back to a default would mis-render a whole manuscript without
    /// saying so. A style we do not have is refused by name.
    #[test]
    fn an_unknown_style_is_refused_rather_than_quietly_replaced() {
        let error = render_cluster(&[item(GINZBURG)], "no-existe-este-estilo").unwrap_err();

        assert_eq!(error.code, "unknown_style");
        assert!(error.message.contains("no-existe-este-estilo"));
    }

    #[test]
    fn malformed_csl_json_is_refused_with_a_code_the_caller_can_branch_on() {
        let error = render_cluster(&[item("{ esto no es json }")], "apa").unwrap_err();

        assert_eq!(error.code, "invalid_csl_json");
    }

    #[test]
    fn an_empty_cluster_is_refused() {
        assert_eq!(
            render_cluster(&[], "apa").unwrap_err().code,
            "empty_cluster"
        );
    }
}
