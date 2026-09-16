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

/// The biggest a stylesheet may be.
///
/// Real CSL styles run to tens of kilobytes; the largest in the official
/// repository is well under this. A cap is here because a style arrives as a
/// file someone chose, and a parser handed something enormous is a parser
/// spending the afternoon on it.
pub const MAX_STYLE_BYTES: usize = 2 * 1024 * 1024;

/// Where a style comes from (§11.6).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StyleSource {
    /// One of the styles bundled with the binary. A fresh install renders
    /// without downloading anything.
    Bundled { name: String },
    /// A `.csl` file the writer supplied. Validated before it is ever used to
    /// render, because a style that fails halfway through a manuscript is
    /// worse than one refused at the door.
    Custom { xml: String },
}

/// What a validated stylesheet says about itself.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StyleInfo {
    pub id: String,
    pub title: String,
}

/// Checks a `.csl` file and reports what it is, without rendering anything.
///
/// §11.6 asks for custom styles to be validated *before* use, so this is the
/// door they come through: it parses the stylesheet and refuses anything that
/// is not an independent CSL style. A dependent style is a pointer to another
/// one and has no rendering of its own, which makes it a perfectly valid file
/// and a useless answer — so it is named as such rather than accepted and then
/// failing later.
pub fn validate_style(xml: &str) -> CslResult<StyleInfo> {
    if xml.len() > MAX_STYLE_BYTES {
        return Err(CslError::new(
            "style_too_large",
            format!("a stylesheet may be at most {MAX_STYLE_BYTES} bytes"),
        ));
    }
    let style = hayagriva::citationberg::Style::from_xml(xml)
        .map_err(|error| CslError::new("invalid_style", format!("{error}")))?;
    match style {
        Style::Independent(style) => Ok(StyleInfo {
            id: style.info.id.clone(),
            title: style.info.title.value.clone(),
        }),
        Style::Dependent(_) => Err(CslError::new(
            "dependent_style",
            "this is a dependent style: it points at another one and cannot render by itself",
        )),
    }
}

/// Resolves a style source into something that can render.
fn style_from(source: &StyleSource) -> CslResult<hayagriva::citationberg::IndependentStyle> {
    match source {
        StyleSource::Bundled { name } => style_by_name(name),
        StyleSource::Custom { xml } => {
            // Validated here too, not only at the door: a style can reach this
            // point from a stored setting written by an older build, and
            // trusting that it was checked once is how an unparseable file
            // becomes a crash halfway through a bibliography.
            validate_style(xml)?;
            match hayagriva::citationberg::Style::from_xml(xml)
                .map_err(|error| CslError::new("invalid_style", format!("{error}")))?
            {
                Style::Independent(style) => Ok(style),
                Style::Dependent(_) => Err(CslError::new(
                    "dependent_style",
                    "this is a dependent style and cannot render by itself",
                )),
            }
        }
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
pub fn render_cluster(items: &[ClusterItem], source: &StyleSource) -> CslResult<RenderedCluster> {
    if items.is_empty() {
        return Err(CslError::new("empty_cluster", "a cluster needs an item"));
    }
    let style = style_from(source)?;
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
    // `{:#}`, not `to_string()`. hayagriva's `Display` writes VT100 escape
    // codes unless the alternate flag is set, so the plain rendering sits
    // behind `{:#}` — the opposite of what one would assume, and how a
    // terminal reset code ended up printed inside citations in a manuscript.
    let mut text = format!("{:#}", first.citation);
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

    let prefix = items.first().and_then(|i| i.prefix.as_deref()).unwrap_or("");
    let suffix = items.first().and_then(|i| i.suffix.as_deref()).unwrap_or("");
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
    let text = format!("{:#}", rendered.citations.first()?.citation);
    let trimmed = text.trim().trim_matches(['(', ')', '[', ']']).trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Renders every citation of a manuscript in one pass (§11.5).
///
/// # Why the whole document at once
///
/// Disambiguation is a property of the manuscript, not of a citation. Two works
/// by the same author in the same year must read `(Acha, 2015a)` and
/// `(Acha, 2015b)`, and which is which depends on every *other* citation in the
/// article — a cluster rendered on its own cannot know that another paragraph
/// cites Acha 2015 too, so rendering one at a time can only ever produce
/// `(Acha, 2015)` twice.
///
/// One driver sees them all, so the style's own disambiguation rules apply.
/// The answers come back in the order the clusters were given.
pub fn render_document(
    clusters: &[Vec<ClusterItem>],
    source: &StyleSource,
) -> CslResult<Vec<RenderedCluster>> {
    if clusters.is_empty() {
        return Ok(Vec::new());
    }
    let style = style_from(source)?;
    let locales = locales();

    // Parsed up front and held, because every `CitationItem` borrows its entry
    // and they all have to outlive the one driver.
    let entries: Vec<Vec<json::Item>> = clusters
        .iter()
        .map(|cluster| {
            cluster
                .iter()
                .map(|item| parse_item(&item.csl_json))
                .collect::<CslResult<Vec<_>>>()
        })
        .collect::<CslResult<_>>()?;

    let mut driver = BibliographyDriver::new();
    for (cluster, parsed) in clusters.iter().zip(&entries) {
        let items: Vec<CitationItem<json::Item>> = parsed
            .iter()
            .zip(cluster)
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
        driver.citation(CitationRequest::from_items(items, &style, &locales));
    }

    let rendered = driver.finish(BibliographyRequest {
        style: &style,
        locale: None,
        locale_files: &locales,
    });

    let mut out = Vec::with_capacity(clusters.len());
    for (index, cluster) in clusters.iter().enumerate() {
        // A citation with no works has nothing to render. It reaches here from
        // a manuscript written before a citation could hold more than one work,
        // and answering with an empty rendering lets the caller leave that
        // citation exactly as it found it. Indexing into it instead took down
        // the whole runtime thread, which is never the right answer to a
        // document being older than the code that reads it.
        if cluster.is_empty() {
            out.push(RenderedCluster {
                text: String::new(),
                author_suppressed: false,
            });
            continue;
        }
        let citation = rendered.citations.get(index).ok_or_else(|| {
            CslError::new("render_failed", format!("no citation for cluster {index}"))
        })?;
        let mut text = format!("{:#}", citation.citation);
        let mut author_suppressed = false;

        // Suppression before affixes: a prefix belongs in front of what is
        // left, not in front of a name that is about to be removed.
        if cluster.len() == 1 && cluster[0].suppress_author {
            if let Some(author) = author_only(&entries[index][0], &style, &locales) {
                if let Some(without) = super::suppress::without_author(&text, &author) {
                    text = without;
                    author_suppressed = true;
                }
            }
        }

        if let Some(first) = cluster.first() {
            text = super::affix::apply_affixes(
                &text,
                first.prefix.as_deref().unwrap_or(""),
                first.suffix.as_deref().unwrap_or(""),
            );
        }

        out.push(RenderedCluster {
            text,
            author_suppressed,
        });
    }

    Ok(out)
}

/// The bibliography, containing only works that were cited (§11.6, criterion 17).
///
/// A derived view, not a stored list: it is built from the citations handed in,
/// so a work that stops being cited stops appearing without anything having to
/// remember to remove it.
pub fn render_bibliography(cited: &[String], source: &StyleSource) -> CslResult<Vec<String>> {
    let style = style_from(source)?;
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
                // Plain, not VT100. See the note in `render_cluster`.
                .map(|item| format!("{:#}", item.content))
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

    fn bundled(name: &str) -> StyleSource {
        StyleSource::Bundled {
            name: name.to_string(),
        }
    }

    fn item(csl_json: &str) -> ClusterItem {
        ClusterItem {
            csl_json: csl_json.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn renders_a_citation_from_zotero_json_with_no_conversion_layer() {
        let out = render_cluster(&[item(GINZBURG)], &bundled("apa")).expect("render");

        assert!(out.text.contains("Ginzburg"), "{}", out.text);
        assert!(out.text.contains("1976"), "{}", out.text);
    }

    /// Criterion 15: several works cited together are one citation, not two.
    #[test]
    fn a_cluster_of_two_works_renders_as_one_citation() {
        let out = render_cluster(&[item(GINZBURG), item(DARNTON)], &bundled("apa")).expect("render");

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
            &bundled("apa"),
        )
        .expect("page");
        assert!(page.text.contains("45"), "{}", page.text);

        let chapter = render_cluster(
            &[ClusterItem {
                locator: Some("3".into()),
                locator_kind: Some("chapter".into()),
                ..item(GINZBURG)
            }],
            &bundled("apa"),
        )
        .expect("chapter");
        assert_ne!(page.text, chapter.text, "the locator kind changed nothing");
    }

    /// Criterion 16: changing style re-renders, which is why nothing stores the
    /// rendered string.
    #[test]
    fn switching_style_re_renders_the_same_citation() {
        let apa = render_cluster(&[item(GINZBURG)], &bundled("apa")).expect("apa");
        let chicago =
            render_cluster(&[item(GINZBURG)], &bundled("chicago-author-date")).expect("chicago");

        assert_ne!(apa.text, chicago.text);
        assert!(chicago.text.contains("Ginzburg"), "{}", chicago.text);
    }

    /// Criterion 17: a derived view of what was cited, never a stored list.
    #[test]
    fn the_bibliography_holds_only_the_works_that_were_cited() {
        let entries = render_bibliography(&[GINZBURG.to_string()], &bundled("apa")).expect("bibliography");

        assert_eq!(entries.len(), 1);
        assert!(entries[0].contains("Ginzburg"), "{}", entries[0]);
        assert!(!entries[0].contains("Darnton"), "{}", entries[0]);
    }

    #[test]
    fn a_work_that_stops_being_cited_stops_appearing() {
        let both = render_bibliography(&[GINZBURG.to_string(), DARNTON.to_string()], &bundled("apa"))
            .expect("both");
        let one = render_bibliography(&[GINZBURG.to_string()], &bundled("apa")).expect("one");

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
            &bundled("apa"),
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
        let out = render_cluster(&[item(GINZBURG)], &bundled("apa")).expect("render");

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
            &bundled("apa"),
        )
        .expect("render");

        assert!(out.text.contains("ver"), "{}", out.text);
        assert!(out.text.contains("y ss."), "{}", out.text);
    }

    /// Falling back to a default would mis-render a whole manuscript without
    /// saying so. A style we do not have is refused by name.
    #[test]
    fn an_unknown_style_is_refused_rather_than_quietly_replaced() {
        let error = render_cluster(&[item(GINZBURG)], &bundled("no-existe-este-estilo")).unwrap_err();

        assert_eq!(error.code, "unknown_style");
        assert!(error.message.contains("no-existe-este-estilo"));
    }

    #[test]
    fn malformed_csl_json_is_refused_with_a_code_the_caller_can_branch_on() {
        let error = render_cluster(&[item("{ esto no es json }")], &bundled("apa")).unwrap_err();

        assert_eq!(error.code, "invalid_csl_json");
    }

    #[test]
    fn an_empty_cluster_is_refused() {
        assert_eq!(
            render_cluster(&[], &bundled("apa")).unwrap_err().code,
            "empty_cluster"
        );
    }
}

#[cfg(test)]
mod style_tests {
    use super::*;

    /// A minimal but real CSL style. Small enough to read, complete enough that
    /// the parser accepts it — which is the point: validation has to accept
    /// valid files, not only reject invalid ones.
    const MINIMAL: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<style xmlns="http://purl.org/net/xbiblio/csl" class="in-text" version="1.0">
  <info>
    <title>Estilo de prueba</title>
    <id>http://example.org/estilo-de-prueba</id>
    <updated>2026-01-01T00:00:00+00:00</updated>
  </info>
  <citation>
    <layout prefix="(" suffix=")">
      <text variable="title"/>
    </layout>
  </citation>
</style>"#;

    const GINZBURG: &str = r#"{
        "id": "ginzburg1976",
        "type": "book",
        "title": "Il formaggio e i vermi",
        "author": [{ "family": "Ginzburg", "given": "Carlo" }],
        "issued": { "date-parts": [[1976]] },
        "language": "it"
    }"#;

    #[test]
    fn a_valid_stylesheet_is_accepted_and_names_itself() {
        let info = validate_style(MINIMAL).expect("valid");

        assert_eq!(info.title, "Estilo de prueba");
        assert_eq!(info.id, "http://example.org/estilo-de-prueba");
    }

    /// §11.6: validated *before* use. A style that fails halfway through a
    /// manuscript is worse than one refused at the door.
    #[test]
    fn a_file_that_is_not_a_stylesheet_is_refused_at_the_door() {
        assert_eq!(
            validate_style("esto no es xml").unwrap_err().code,
            "invalid_style"
        );
        assert_eq!(
            validate_style("<style></style>").unwrap_err().code,
            "invalid_style"
        );
    }

    /// A parser handed something enormous is a parser spending the afternoon
    /// on it, and a style arrives as a file somebody chose.
    #[test]
    fn an_enormous_file_is_refused_without_being_parsed() {
        let huge = "<".repeat(MAX_STYLE_BYTES + 1);

        assert_eq!(validate_style(&huge).unwrap_err().code, "style_too_large");
    }

    #[test]
    fn a_validated_stylesheet_can_then_render() {
        let out = render_cluster(
            &[ClusterItem {
                csl_json: GINZBURG.to_string(),
                ..Default::default()
            }],
            &StyleSource::Custom {
                xml: MINIMAL.to_string(),
            },
        )
        .expect("render");

        assert!(out.text.contains("formaggio"), "{}", out.text);
    }

    /// Reaching the renderer is not proof of having been checked: a style can
    /// arrive from a setting written by an older build. Trusting that it was
    /// validated once is how an unparseable file becomes a crash halfway
    /// through a bibliography.
    #[test]
    fn a_custom_style_is_checked_again_when_it_is_used() {
        let error = render_cluster(
            &[ClusterItem {
                csl_json: GINZBURG.to_string(),
                ..Default::default()
            }],
            &StyleSource::Custom {
                xml: "ya no es un estilo".to_string(),
            },
        )
        .unwrap_err();

        assert_eq!(error.code, "invalid_style");
    }

    #[test]
    fn the_bundled_set_still_works_alongside_custom_ones() {
        let out = render_cluster(
            &[ClusterItem {
                csl_json: GINZBURG.to_string(),
                ..Default::default()
            }],
            &StyleSource::Bundled {
                name: "apa".to_string(),
            },
        )
        .expect("render");

        assert!(out.text.contains("Ginzburg"), "{}", out.text);
    }
}

#[cfg(test)]
mod output_tests {
    use super::*;

    const GINZBURG: &str = r#"{
        "id": "g",
        "type": "book",
        "title": "Il formaggio e i vermi",
        "author": [{ "family": "Ginzburg", "given": "Carlo" }],
        "issued": { "date-parts": [[1976]] },
        "language": "it"
    }"#;

    fn apa() -> StyleSource {
        StyleSource::Bundled { name: "apa".into() }
    }

    fn only(csl: &str) -> Vec<ClusterItem> {
        vec![ClusterItem {
            csl_json: csl.to_string(),
            ..Default::default()
        }]
    }

    /// hayagriva's `Display` writes VT100 escape codes unless the alternate
    /// flag is set. Rendering with `to_string()` put a terminal reset code
    /// inside citations in a real manuscript; this is the test that keeps the
    /// output meant for a page from being the output meant for a console.
    #[test]
    fn a_rendered_citation_carries_no_terminal_escape_codes() {
        let out = render_cluster(&only(GINZBURG), &apa()).expect("render");

        assert!(!out.text.contains('\u{1b}'), "escape code in {:?}", out.text);
        assert!(!out.text.contains("[0m"), "reset code in {:?}", out.text);
    }

    #[test]
    fn a_rendered_bibliography_carries_none_either() {
        let entries = render_bibliography(&[GINZBURG.to_string()], &apa()).expect("bibliography");

        for entry in &entries {
            assert!(!entry.contains('\u{1b}'), "escape code in {entry:?}");
            assert!(!entry.contains("[0m"), "reset code in {entry:?}");
        }
    }

    /// Suppression reads the author-only rendering too, so it has the same
    /// hazard and needs the same guarantee.
    #[test]
    fn suppressing_the_author_leaves_no_escape_codes_behind() {
        let out = render_cluster(
            &[ClusterItem {
                csl_json: GINZBURG.to_string(),
                locator: Some("45".into()),
                suppress_author: true,
                ..Default::default()
            }],
            &apa(),
        )
        .expect("render");

        assert!(out.author_suppressed, "not suppressed: {:?}", out.text);
        assert!(!out.text.contains('\u{1b}'), "escape code in {:?}", out.text);
    }

    /// The citation still has to read correctly; stripping formatting must not
    /// have taken the words with it.
    #[test]
    fn the_plain_rendering_is_still_a_citation() {
        let out = render_cluster(&only(GINZBURG), &apa()).expect("render");

        assert!(out.text.contains("Ginzburg"), "{}", out.text);
        assert!(out.text.contains("1976"), "{}", out.text);
    }
}

#[cfg(test)]
mod disambiguation_tests {
    use super::*;

    fn work(id: &str, title: &str) -> String {
        format!(
            r#"{{"id":"{id}","type":"book","title":"{title}",
                "author":[{{"family":"Acha","given":"Omar"}}],
                "issued":{{"date-parts":[[2015]]}},"language":"es"}}"#
        )
    }

    fn cluster(csl: &str) -> Vec<ClusterItem> {
        vec![ClusterItem {
            csl_json: csl.to_string(),
            ..Default::default()
        }]
    }

    fn apa() -> StyleSource {
        StyleSource::Bundled { name: "apa".into() }
    }

    /// Two works by one author in one year have to be told apart, and the
    /// letters depend on every other citation in the manuscript — which is why
    /// they are rendered together rather than one at a time.
    #[test]
    fn two_works_by_one_author_in_one_year_get_their_letters() {
        let out = render_document(
            &[
                cluster(&work("a", "Lucha y organizacion")),
                cluster(&work("b", "Un revisionismo historico")),
            ],
            &apa(),
        )
        .expect("render");

        assert_eq!(out.len(), 2);
        assert_ne!(out[0].text, out[1].text, "both rendered the same: {:?}", out);
        assert!(out[0].text.contains("2015a"), "{:?}", out[0].text);
        assert!(out[1].text.contains("2015b"), "{:?}", out[1].text);
    }

    /// The letters are shared across the manuscript, so two works cited in one
    /// cluster are told apart there too.
    #[test]
    fn works_cited_together_are_told_apart_inside_the_cluster() {
        let out = render_document(
            &[vec![
                ClusterItem {
                    csl_json: work("a", "Lucha y organizacion"),
                    ..Default::default()
                },
                ClusterItem {
                    csl_json: work("b", "Un revisionismo historico"),
                    ..Default::default()
                },
            ]],
            &apa(),
        )
        .expect("render");

        assert!(out[0].text.contains("2015a"), "{:?}", out[0].text);
        assert!(out[0].text.contains("2015b"), "{:?}", out[0].text);
    }

    /// A single work needs no letter, and adding one would be wrong.
    #[test]
    fn a_work_with_no_twin_keeps_its_bare_year() {
        let out = render_document(&[cluster(&work("a", "Lucha y organizacion"))], &apa())
            .expect("render");

        assert!(out[0].text.contains("2015"), "{:?}", out[0].text);
        assert!(!out[0].text.contains("2015a"), "{:?}", out[0].text);
    }

    #[test]
    fn the_answers_come_back_in_the_order_they_were_asked() {
        let out = render_document(
            &[
                cluster(&work("a", "Primero")),
                cluster(r#"{"id":"g","type":"book","title":"Il formaggio",
                    "author":[{"family":"Ginzburg","given":"Carlo"}],
                    "issued":{"date-parts":[[1976]]},"language":"it"}"#),
            ],
            &apa(),
        )
        .expect("render");

        assert!(out[0].text.contains("Acha"), "{:?}", out[0].text);
        assert!(out[1].text.contains("Ginzburg"), "{:?}", out[1].text);
    }

    #[test]
    fn an_empty_manuscript_renders_nothing_rather_than_failing() {
        assert_eq!(render_document(&[], &apa()).expect("render"), Vec::new());
    }
}

#[cfg(test)]
mod empty_cluster_tests {
    use super::*;

    fn apa() -> StyleSource {
        StyleSource::Bundled { name: "apa".into() }
    }

    const GINZBURG: &str = r#"{"id":"g","type":"book","title":"Il formaggio",
        "author":[{"family":"Ginzburg","given":"Carlo"}],
        "issued":{"date-parts":[[1976]]},"language":"it"}"#;

    /// A citation with no works reaches the engine from a manuscript written
    /// before a citation could hold more than one. Indexing into it took down
    /// the runtime thread — which is never the right answer to a document being
    /// older than the code reading it.
    #[test]
    fn an_empty_cluster_answers_instead_of_bringing_down_the_thread() {
        let out = render_document(&[vec![]], &apa()).expect("render");

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "");
    }

    /// And it must not take its neighbours with it: the rest of the manuscript
    /// still renders, in order.
    #[test]
    fn an_empty_cluster_does_not_disturb_the_ones_around_it() {
        let cited = vec![ClusterItem {
            csl_json: GINZBURG.to_string(),
            ..Default::default()
        }];

        let out = render_document(&[vec![], cited.clone(), vec![]], &apa()).expect("render");

        assert_eq!(out.len(), 3);
        assert_eq!(out[0].text, "");
        assert!(out[1].text.contains("Ginzburg"), "{:?}", out[1].text);
        assert_eq!(out[2].text, "");
    }
}

#[cfg(test)]
mod real_world_tests {
    use super::*;

    fn apa() -> StyleSource {
        StyleSource::Bundled { name: "apa".into() }
    }

    /// Zotero's local API shapes its ids as URIs, not bare keys. If that alone
    /// defeated disambiguation, every letter would be missing in the app while
    /// present in a hand-written fixture — so it is worth knowing which.
    fn zotero_item(key: &str, title: &str) -> String {
        format!(
            r#"{{"id":"http://zotero.org/users/local/abc/items/{key}",
                "type":"article-journal","title":"{title}",
                "author":[{{"family":"Nieto","given":"Agustin"}}],
                "issued":{{"date-parts":[["2022"]]}}}}"#
        )
    }

    fn one(csl: &str) -> Vec<ClusterItem> {
        vec![ClusterItem {
            csl_json: csl.to_string(),
            ..Default::default()
        }]
    }

    /// The shape the application actually sends: URI ids, a string year rather
    /// than a number, no `language`.
    #[test]
    fn disambiguation_survives_the_shape_zotero_really_sends() {
        let out = render_document(
            &[
                one(&zotero_item("AAAA1111", "Primer articulo")),
                one(&zotero_item("BBBB2222", "Segundo articulo")),
            ],
            &apa(),
        )
        .expect("render");

        assert_ne!(out[0].text, out[1].text, "both rendered the same: {out:?}");
        assert!(out[0].text.contains("2022a"), "{:?}", out[0].text);
        assert!(out[1].text.contains("2022b"), "{:?}", out[1].text);
    }

    /// Two different works of one author and year, cited side by side in one
    /// citation. This is the case on screen.
    #[test]
    fn two_works_of_one_year_inside_one_cluster_get_their_letters() {
        let out = render_document(
            &[vec![
                ClusterItem {
                    csl_json: zotero_item("AAAA1111", "Primer articulo"),
                    ..Default::default()
                },
                ClusterItem {
                    csl_json: zotero_item("BBBB2222", "Segundo articulo"),
                    ..Default::default()
                },
            ]],
            &apa(),
        )
        .expect("render");

        assert!(out[0].text.contains("2022a"), "{:?}", out[0].text);
        assert!(out[0].text.contains("2022b"), "{:?}", out[0].text);
    }

    /// The same work cited twice in one citation. There are no letters to add
    /// — it is one work — so what the reader sees is a repetition, which is
    /// what the deduplication upstream exists to prevent.
    #[test]
    fn the_same_work_twice_has_no_letters_to_add() {
        let same = zotero_item("AAAA1111", "Un articulo");
        let out = render_document(
            &[vec![
                ClusterItem {
                    csl_json: same.clone(),
                    ..Default::default()
                },
                ClusterItem {
                    csl_json: same,
                    ..Default::default()
                },
            ]],
            &apa(),
        )
        .expect("render");

        assert!(!out[0].text.contains("2022a"), "letters for one work: {:?}", out[0].text);
    }
}
