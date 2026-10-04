//! Research-chat scope: Corpus, Biblioteca (the synced Zotero libraries) or
//! both.
//!
//! The two retrieval legs never meet numerically. Corpus candidates carry
//! reranker/RRF scores and bibliography passages carry cosine similarity;
//! those scales say nothing about each other, so the merge below looks only at
//! RANK. See [`merge_scopes`] for the exact rule.

use rusqlite::Connection;
use serde::Deserialize;

use super::params::RagParams;
use super::{RagBibliographyLocation, RagBibliographySource, RagSource};
use crate::bibliography::retrieval::{PassageHit, WorkFilters};

/// Where a question looks. Anything but the explicit values is rejected, so a
/// typo in the frontend fails loudly instead of silently searching the corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RagScope {
    #[default]
    Corpus,
    Biblioteca,
    Both,
}

impl RagScope {
    pub(crate) fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            None | Some("corpus") => Ok(Self::Corpus),
            Some("biblioteca") => Ok(Self::Biblioteca),
            Some("both") => Ok(Self::Both),
            Some(other) => Err(format!(
                "Alcance del chat no soportado: {other}. Usá 'corpus', 'biblioteca' o 'both'."
            )),
        }
    }

    pub(crate) fn includes_corpus(self) -> bool {
        matches!(self, Self::Corpus | Self::Both)
    }

    pub(crate) fn includes_bibliography(self) -> bool {
        matches!(self, Self::Biblioteca | Self::Both)
    }
}

/// One Zotero library as the UI names it (`user`/`group` + native id).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RagLibraryRef {
    pub library_type: String,
    pub library_id: String,
}

/// Why the Biblioteca leg contributed nothing, said honestly to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BibliographyNotice {
    /// No (selected) library has been synced into the catalog.
    NoLibrarySynced,
    /// Libraries exist but there is no active embedding generation.
    NoEmbeddings,
    /// The query could not be embedded (provider down, no key...).
    EmbeddingUnavailable,
    /// Any other failure of the leg; the corpus answer still stands.
    Failed,
}

impl BibliographyNotice {
    /// Stable machine string the frontend turns into words.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::NoLibrarySynced => "no_library_synced",
            Self::NoEmbeddings => "no_embeddings",
            Self::EmbeddingUnavailable => "embedding_unavailable",
            Self::Failed => "failed",
        }
    }
}

pub(crate) struct BibliographyLeg {
    pub sources: Vec<RagSource>,
    pub notice: Option<BibliographyNotice>,
    /// Short, secret-free cause behind a [`BibliographyNotice::Failed`] or
    /// [`BibliographyNotice::EmbeddingUnavailable`] notice: what the user (and
    /// the app log) need to tell a SQL fault from a provider fault. `None`
    /// for the notices that need no explanation.
    pub detail: Option<String>,
}

/// Longest cause carried to the UI and the log: enough for an error message,
/// never a dump.
const DETAIL_MAX_CHARS: usize = 220;

/// A cause made safe to show and to log: one line, bounded, credentials
/// redacted by the same rules the app log applies to everything it writes.
pub(crate) fn short_cause(error: &str) -> String {
    let one_line = error.split_whitespace().collect::<Vec<_>>().join(" ");
    crate::app_logs::sanitize_field(one_line, DETAIL_MAX_CHARS)
}

/// "p. 3", "pp. 3–4", "párr. 2" or "párr. 2–3". The label the prompt shows
/// the model; the UI renders the same structure with its own translations.
pub(crate) fn location_label(location: &RagBibliographyLocation) -> String {
    let range = if location.from == location.to {
        location.from.to_string()
    } else {
        format!("{}–{}", location.from, location.to)
    };
    match (location.kind.as_str(), location.from == location.to) {
        ("paragraphs", _) => format!("párr. {range}"),
        (_, true) => format!("p. {range}"),
        (_, false) => format!("pp. {range}"),
    }
}

/// Header of one numbered fragment in the prompt. Corpus keeps its historic
/// `«title» (collection)`; a bibliography passage names the work and where in
/// it the text sits.
pub(crate) fn fragment_header(source: &RagSource) -> String {
    let Some(meta) = &source.bibliography else {
        return format!("«{}» ({})", source.item_title, source.collection_name);
    };
    let mut parts: Vec<String> = Vec::new();
    if !meta.authors.trim().is_empty() {
        parts.push(meta.authors.trim().to_string());
    }
    if let Some(year) = meta.year {
        parts.push(year.to_string());
    }
    if let Some(location) = &meta.location {
        parts.push(location_label(location));
    }
    if parts.is_empty() {
        parts.push(meta.library_name.clone());
    }
    format!("«{}» ({})", source.item_title, parts.join(" · "))
}

/// Interleaves the two legs by rank and renumbers.
///
/// Rule (the only place the scopes meet): take corpus rank 1, bibliography
/// rank 1, corpus rank 2, bibliography rank 2, ... When one leg runs out the
/// other fills the rest. Scores are never read, added or compared. The result
/// obeys the same budget the corpus alone obeyed: at most `top_k` sources and
/// at most `context_max_chars` of snippet text, stopping at the first source
/// that would overflow (the first source is truncated instead of dropped, so
/// there is never an empty answer when material exists). Indexes are one
/// continuous 1..n numbering across both scopes, the `[n]` the model cites.
pub(crate) fn merge_scopes(
    corpus: Vec<RagSource>,
    bibliography: Vec<RagSource>,
    top_k: usize,
    context_max_chars: usize,
) -> Vec<RagSource> {
    let mut corpus = corpus.into_iter();
    let mut bibliography = bibliography.into_iter();
    let mut merged: Vec<RagSource> = Vec::new();
    let mut total_chars = 0usize;
    'ranks: loop {
        let (from_corpus, from_bibliography) = (corpus.next(), bibliography.next());
        if from_corpus.is_none() && from_bibliography.is_none() {
            break;
        }
        for mut source in [from_corpus, from_bibliography].into_iter().flatten() {
            if merged.len() >= top_k {
                break 'ranks;
            }
            let chars = source.snippet.chars().count();
            if total_chars + chars > context_max_chars {
                if !merged.is_empty() {
                    break 'ranks;
                }
                source.snippet = source.snippet.chars().take(context_max_chars).collect();
            }
            total_chars += source.snippet.chars().count();
            source.index = (merged.len() + 1) as u32;
            merged.push(source);
        }
    }
    merged
}

/// The bibliography leg: passage search over the chosen libraries (empty
/// `libraries` = every synced one), converted to citable sources. Never fails:
/// a problem becomes a [`BibliographyNotice`] and an empty source list.
pub(crate) fn bibliography_leg(
    conn: &Connection,
    contract_hash: &str,
    query: &str,
    libraries: &[RagLibraryRef],
    params: &RagParams,
    embed: &dyn Fn(&str) -> Result<Vec<f32>, String>,
) -> BibliographyLeg {
    match run_bibliography_leg(conn, contract_hash, query, libraries, params, embed) {
        Ok(leg) => leg,
        Err(error) => failed(BibliographyNotice::Failed, &error),
    }
}

fn nothing(notice: BibliographyNotice) -> BibliographyLeg {
    BibliographyLeg {
        sources: Vec::new(),
        notice: Some(notice),
        detail: None,
    }
}

/// An empty leg that says why: the notice for the UI plus the short cause the
/// caller also writes to the app log.
pub(crate) fn failed(notice: BibliographyNotice, error: &str) -> BibliographyLeg {
    BibliographyLeg {
        sources: Vec::new(),
        notice: Some(notice),
        detail: Some(short_cause(error)),
    }
}

/// Passages kept per work before the global cut: one work never crowds out the
/// rest of the library.
const PASSAGES_PER_WORK: usize = 2;

fn run_bibliography_leg(
    conn: &Connection,
    contract_hash: &str,
    query: &str,
    libraries: &[RagLibraryRef],
    params: &RagParams,
    embed: &dyn Fn(&str) -> Result<Vec<f32>, String>,
) -> Result<BibliographyLeg, String> {
    use crate::bibliography::{generation, retrieval as bib};

    let describe = |error: crate::bibliography::repository::BibliographyError| {
        format!("{}: {}", error.code, error.message)
    };

    // Which libraries: none named = every synced one (no filter), but "every
    // synced one" is nothing when the catalog has no library at all.
    let mut library_ids: Vec<String> = Vec::new();
    if libraries.is_empty() {
        let any: i64 = conn
            .query_row("SELECT COUNT(*) FROM zotero_libraries", [], |row| {
                row.get(0)
            })
            .map_err(|error| format!("Failed to count Zotero libraries: {error}"))?;
        if any == 0 {
            return Ok(nothing(BibliographyNotice::NoLibrarySynced));
        }
    } else {
        for library in libraries {
            library_ids.extend(
                bib::resolve_zotero_library_rows(conn, &library.library_type, &library.library_id)
                    .map_err(describe)?,
            );
        }
        if library_ids.is_empty() {
            return Ok(nothing(BibliographyNotice::NoLibrarySynced));
        }
    }

    // Passage search is vector-only: no active generation means no passages,
    // and the embedder must not even run.
    if generation::active_generation(conn, contract_hash)
        .map_err(describe)?
        .is_none()
    {
        return Ok(nothing(BibliographyNotice::NoEmbeddings));
    }

    let top_k = params.top_k.max(1);
    let filters = WorkFilters {
        library_ids,
        ..WorkFilters::default()
    };
    let hits = match bib::search_passages(
        conn,
        contract_hash,
        query,
        (top_k * 3).max(10),
        PASSAGES_PER_WORK,
        top_k,
        &filters,
        embed,
    ) {
        Ok(hits) => hits,
        Err(error) if error.code == "search_unavailable" => {
            return Ok(failed(
                BibliographyNotice::EmbeddingUnavailable,
                &error.message,
            ));
        }
        Err(error) => return Err(describe(error)),
    };

    let terms = super::retrieval::extract_query_terms(query);
    let mut sources = Vec::with_capacity(hits.len());
    for hit in hits {
        // The similarity floor is the bibliography's own: cosine against
        // cosine, never against the corpus legs.
        if params.min_similarity > 0.0 && hit.vector_score < params.min_similarity {
            continue;
        }
        let Some(display) = bib::read_work_display(conn, &hit.item_id).map_err(describe)? else {
            continue;
        };
        let location = passage_location(conn, &hit);
        let (snippet, _) =
            super::retrieval::snippet_window(&hit.text, &terms, params.snippet_max_chars);
        sources.push(RagSource {
            index: 0,
            asset_id: String::new(),
            item_id: hit.item_id.clone(),
            item_title: hit.title.clone(),
            collection_id: String::new(),
            collection_name: display.library_name.clone(),
            snippet,
            score: hit.vector_score,
            start_seconds: None,
            end_seconds: None,
            provenance: None,
            bibliography: Some(RagBibliographySource {
                chunk_id: hit.chunk_id.clone(),
                item_key: hit.item_key.clone(),
                library_name: display.library_name,
                library_type: display.library_type,
                library_native_id: display.library_native_id,
                authors: display.authors,
                year: display.year,
                location,
            }),
        });
    }
    Ok(BibliographyLeg {
        sources,
        notice: None,
        detail: None,
    })
}

/// Where a passage sits: a page range for a PDF, a paragraph range for an
/// HTML snapshot (stored as one "page", so a page number there would lie).
/// `None` when the attachment or its text is gone: the citation still shows.
fn passage_location(conn: &Connection, hit: &PassageHit) -> Option<RagBibliographyLocation> {
    let first_page = hit.spans.iter().map(|span| span.0).min()?;
    let last_page = hit.spans.iter().map(|span| span.0).max()?;
    let content_type =
        crate::bibliography::attachment::attachment_ref_for(conn, &hit.attachment_id)
            .ok()
            .flatten()
            .and_then(|attachment| attachment.content_type)
            .unwrap_or_default()
            .to_ascii_lowercase();
    if !content_type.contains("html") {
        return Some(RagBibliographyLocation {
            kind: "pages".to_string(),
            from: first_page,
            to: last_page,
        });
    }
    let on_first_page = || hit.spans.iter().filter(|span| span.0 == first_page);
    let start = on_first_page().map(|span| span.1).min()?;
    let end = on_first_page().map(|span| span.2).max()?;
    let text: String = conn
        .query_row(
            "SELECT text_content FROM bibliographic_page_texts
              WHERE attachment_id = ?1 AND page_number = ?2
              ORDER BY CASE method WHEN 'ocr' THEN 0 ELSE 1 END LIMIT 1",
            rusqlite::params![hit.attachment_id, first_page],
            |row| row.get(0),
        )
        .ok()?;
    let (from, to) = crate::bibliography::chunks::paragraph_range(
        &text,
        usize::try_from(start).ok()?,
        usize::try_from(end).ok()?,
    )?;
    Some(RagBibliographyLocation {
        kind: "paragraphs".to_string(),
        from: from as i64,
        to: to as i64,
    })
}

/// One passage as the Writing "Obras" tab lists it: the work it belongs to
/// (enough to cite it), the snippet and where in the work it sits.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PassageResult {
    pub chunk_id: String,
    pub item_id: String,
    pub item_key: String,
    pub title: String,
    pub authors: String,
    pub year: Option<i64>,
    pub library_name: String,
    pub library_type: String,
    pub library_native_id: String,
    /// The catalog's CSL-JSON, what a citation snapshots.
    pub csl_json: String,
    pub snippet: String,
    pub location: Option<RagBibliographyLocation>,
    pub score: f64,
}

pub(crate) struct PassageSearch {
    pub passages: Vec<PassageResult>,
    pub notice: Option<BibliographyNotice>,
    /// See [`BibliographyLeg::detail`].
    pub detail: Option<String>,
}

/// Passage search for the Writing tab: the same leg the chat uses (same
/// libraries, similarity floor, snippet and location rules, same honest
/// notices), shaped as results rather than prompt sources. One source of truth
/// for what a passage is.
pub(crate) fn passage_search(
    conn: &Connection,
    contract_hash: &str,
    query: &str,
    libraries: &[RagLibraryRef],
    params: &RagParams,
    embed: &dyn Fn(&str) -> Result<Vec<f32>, String>,
) -> PassageSearch {
    let leg = bibliography_leg(conn, contract_hash, query, libraries, params, embed);
    let passages = leg
        .sources
        .into_iter()
        .filter_map(|source| {
            let meta = source.bibliography?;
            let csl_json = crate::bibliography::retrieval::read_work_display(conn, &source.item_id)
                .ok()
                .flatten()
                .map(|display| display.csl_json)
                .unwrap_or_default();
            Some(PassageResult {
                chunk_id: meta.chunk_id,
                item_id: source.item_id,
                item_key: meta.item_key,
                title: source.item_title,
                authors: meta.authors,
                year: meta.year,
                library_name: meta.library_name,
                library_type: meta.library_type,
                library_native_id: meta.library_native_id,
                csl_json,
                snippet: source.snippet,
                location: meta.location,
                score: source.score,
            })
        })
        .collect();
    PassageSearch {
        passages,
        notice: leg.notice,
        detail: leg.detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bibliography::retrieval::passage_tests::{
        activate_gen, passage_db, seed_chunk, seed_item, CONTRACT,
    };

    fn source(title: &str, score: f64, snippet: &str) -> RagSource {
        RagSource {
            index: 99,
            asset_id: "a".into(),
            item_id: "i".into(),
            item_title: title.into(),
            collection_id: "c".into(),
            collection_name: "Archivo".into(),
            snippet: snippet.into(),
            score,
            start_seconds: None,
            end_seconds: None,
            provenance: None,
            bibliography: None,
        }
    }

    fn biblio(title: &str, score: f64, snippet: &str) -> RagSource {
        let mut s = source(title, score, snippet);
        s.asset_id = String::new();
        s.bibliography = Some(RagBibliographySource {
            chunk_id: format!("chunk-{title}"),
            item_key: "K1".into(),
            library_name: "Mi biblioteca".into(),
            library_type: "user".into(),
            library_native_id: "0".into(),
            authors: "Bloch, Febvre".into(),
            year: Some(1949),
            location: Some(RagBibliographyLocation {
                kind: "pages".into(),
                from: 3,
                to: 3,
            }),
        });
        s
    }

    fn titles(sources: &[RagSource]) -> Vec<String> {
        sources.iter().map(|s| s.item_title.clone()).collect()
    }

    // ── scope parsing ────────────────────────────────────────────────────

    #[test]
    fn scope_defaults_to_corpus_and_rejects_unknown_values() {
        assert_eq!(RagScope::parse(None).unwrap(), RagScope::Corpus);
        assert_eq!(RagScope::parse(Some("")).unwrap(), RagScope::Corpus);
        assert_eq!(RagScope::parse(Some("corpus")).unwrap(), RagScope::Corpus);
        assert_eq!(
            RagScope::parse(Some("biblioteca")).unwrap(),
            RagScope::Biblioteca
        );
        assert_eq!(RagScope::parse(Some("both")).unwrap(), RagScope::Both);
        assert!(RagScope::parse(Some("todo")).is_err());
    }

    #[test]
    fn scope_says_which_legs_run() {
        assert!(RagScope::Corpus.includes_corpus() && !RagScope::Corpus.includes_bibliography());
        assert!(
            !RagScope::Biblioteca.includes_corpus() && RagScope::Biblioteca.includes_bibliography()
        );
        assert!(RagScope::Both.includes_corpus() && RagScope::Both.includes_bibliography());
    }

    // ── merge: ranks, never scores ───────────────────────────────────────

    #[test]
    fn merge_interleaves_by_rank_ignoring_score_scales() {
        // Bibliography scores dwarf the corpus ones; the order must not care.
        let corpus = vec![source("c1", 0.01, "x"), source("c2", 0.009, "x")];
        let bib = vec![biblio("b1", 0.99, "x"), biblio("b2", 0.98, "x")];
        let merged = merge_scopes(corpus, bib, 10, 10_000);
        assert_eq!(titles(&merged), ["c1", "b1", "c2", "b2"]);
        // Scores travel untouched: nothing was added or normalised.
        assert_eq!(merged[0].score, 0.01);
        assert_eq!(merged[1].score, 0.99);
    }

    #[test]
    fn merge_numbers_continuously_across_both_scopes() {
        let merged = merge_scopes(
            vec![source("c1", 1.0, "x")],
            vec![biblio("b1", 1.0, "x"), biblio("b2", 1.0, "x")],
            10,
            10_000,
        );
        let indexes: Vec<u32> = merged.iter().map(|s| s.index).collect();
        assert_eq!(indexes, [1, 2, 3]);
        assert_eq!(titles(&merged), ["c1", "b1", "b2"], "the longer leg fills");
    }

    #[test]
    fn merge_respects_top_k_and_keeps_the_fixed_share() {
        let corpus: Vec<_> = (1..=5)
            .map(|n| source(&format!("c{n}"), 1.0, "x"))
            .collect();
        let bib: Vec<_> = (1..=5)
            .map(|n| biblio(&format!("b{n}"), 1.0, "x"))
            .collect();
        let merged = merge_scopes(corpus, bib, 4, 10_000);
        assert_eq!(titles(&merged), ["c1", "b1", "c2", "b2"]);
    }

    #[test]
    fn merge_keeps_the_context_budget_of_the_corpus_alone() {
        let big = "y".repeat(60);
        let merged = merge_scopes(
            vec![source("c1", 1.0, &big)],
            vec![biblio("b1", 1.0, &big), biblio("b2", 1.0, &big)],
            10,
            130,
        );
        // 60 + 60 fit; the third would reach 180 > 130 and is not listed.
        assert_eq!(titles(&merged), ["c1", "b1"]);
    }

    #[test]
    fn merge_truncates_an_oversized_first_source_instead_of_dropping_it() {
        let merged = merge_scopes(
            Vec::new(),
            vec![biblio("b1", 1.0, &"z".repeat(500))],
            5,
            100,
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].snippet.chars().count(), 100);
    }

    #[test]
    fn merge_with_one_empty_leg_is_that_leg_renumbered() {
        let only_corpus = merge_scopes(
            vec![source("c1", 1.0, "x"), source("c2", 1.0, "x")],
            Vec::new(),
            10,
            1_000,
        );
        assert_eq!(titles(&only_corpus), ["c1", "c2"]);
        assert_eq!(only_corpus[1].index, 2);
        let only_bib = merge_scopes(Vec::new(), vec![biblio("b1", 1.0, "x")], 10, 1_000);
        assert_eq!(titles(&only_bib), ["b1"]);
        assert_eq!(only_bib[0].index, 1);
    }

    // ── labels ────────────────────────────────────────────────────────────

    fn loc(kind: &str, from: i64, to: i64) -> RagBibliographyLocation {
        RagBibliographyLocation {
            kind: kind.into(),
            from,
            to,
        }
    }

    #[test]
    fn location_label_distinguishes_pages_from_paragraphs() {
        assert_eq!(location_label(&loc("pages", 3, 3)), "p. 3");
        assert_eq!(location_label(&loc("pages", 3, 4)), "pp. 3–4");
        assert_eq!(location_label(&loc("paragraphs", 2, 2)), "párr. 2");
        assert_eq!(location_label(&loc("paragraphs", 2, 3)), "párr. 2–3");
    }

    #[test]
    fn fragment_header_keeps_corpus_format_and_names_the_work_for_bibliography() {
        let corpus = source("Acta", 1.0, "x");
        assert_eq!(fragment_header(&corpus), "«Acta» (Archivo)");
        let bib = biblio("Apología", 1.0, "x");
        assert_eq!(
            fragment_header(&bib),
            "«Apología» (Bloch, Febvre · 1949 · p. 3)"
        );
    }

    #[test]
    fn fragment_header_survives_missing_authors_year_and_location() {
        let mut bib = biblio("Sin datos", 1.0, "x");
        let meta = bib.bibliography.as_mut().unwrap();
        meta.authors = String::new();
        meta.year = None;
        meta.location = None;
        assert_eq!(fragment_header(&bib), "«Sin datos» (Mi biblioteca)");
    }

    // ── persistence: the stored shape round-trips and stays compatible ───

    #[test]
    fn bibliography_source_round_trips_through_json() {
        let original = biblio("b1", 0.5, "texto");
        let json = serde_json::to_string(&original).unwrap();
        let back: RagSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back.bibliography, original.bibliography);
        assert_eq!(back.snippet, "texto");
    }

    #[test]
    fn corpus_source_json_has_no_bibliography_key_and_old_json_still_loads() {
        let json = serde_json::to_value(source("c1", 1.0, "x")).unwrap();
        assert!(json.get("bibliography").is_none());
        let old: RagSource = serde_json::from_value(serde_json::json!({
            "index": 1, "assetId": "a", "itemId": "i", "itemTitle": "t",
            "collectionId": "c", "collectionName": "n", "snippet": "s", "score": 1.0,
            "startSeconds": null, "endSeconds": null
        }))
        .unwrap();
        assert!(old.bibliography.is_none());
    }

    // ── bibliography leg against a real catalog ──────────────────────────

    const QUERY_VECTOR: [f32; 4] = [0.0, 1.0, 0.0, 0.0];

    fn embed_ok(_: &str) -> Result<Vec<f32>, String> {
        Ok(QUERY_VECTOR.to_vec())
    }

    fn seed_profile(conn: &Connection, item_id: &str, text: &str) {
        let hash = crate::bibliography::profile::profile_input_hash(text);
        conn.execute(
            "INSERT INTO bibliographic_semantic_profiles
               (item_id, profile_revision, template_version, canonical_text,
                input_hash, field_provenance_json, created_at, updated_at)
             VALUES (?1, 1, 'bibliography-profile-v1', ?2, ?3, '[]', 1, 1)",
            rusqlite::params![item_id, text, hash],
        )
        .expect("seed profile");
    }

    /// Two libraries (a, b), one work each with one embedded passage.
    fn two_library_catalog() -> (Connection, String, String) {
        let mut conn = passage_db();
        let (_la, item_a) = seed_item(&mut conn, "pa", "a", "KA0001", "Obra A");
        let (_lb, item_b) = seed_item(&mut conn, "pb", "b", "KB0001", "Obra B");
        seed_profile(&conn, &item_a, "Obra A con vocabulario compartido.");
        seed_profile(&conn, &item_b, "Obra B con vocabulario compartido.");
        let gen = activate_gen(&mut conn, "gen-scope");
        let chunk_a = seed_chunk(
            &conn,
            &item_a,
            0,
            "Texto de la obra A.",
            3,
            &gen,
            &[0.0, 1.0, 0.0, 0.0],
            None,
        );
        let chunk_b = seed_chunk(
            &conn,
            &item_b,
            0,
            "Texto de la obra B.",
            5,
            &gen,
            &[0.0, 0.6, 0.8, 0.0],
            None,
        );
        (conn, chunk_a, chunk_b)
    }

    fn params() -> RagParams {
        RagParams::default()
    }

    #[test]
    fn leg_turns_passages_into_sources_with_work_and_pages() {
        let (conn, chunk_a, _) = two_library_catalog();
        let leg = bibliography_leg(&conn, CONTRACT, "compartido", &[], &params(), &embed_ok);
        assert_eq!(leg.notice, None);
        assert_eq!(leg.sources.len(), 2);
        let first = &leg.sources[0];
        let meta = first.bibliography.as_ref().expect("bibliography meta");
        assert_eq!(meta.chunk_id, chunk_a);
        assert_eq!(meta.item_key, "KA0001");
        assert_eq!(meta.library_type, "user");
        assert_eq!(meta.library_native_id, "lib-a");
        assert_eq!(meta.year, Some(2020));
        assert_eq!(
            meta.location,
            Some(RagBibliographyLocation {
                kind: "pages".into(),
                from: 3,
                to: 3
            })
        );
        assert_eq!(first.item_title, "Obra A");
        assert_eq!(first.snippet, "Texto de la obra A.");
        assert!(first.asset_id.is_empty(), "no corpus asset behind it");
    }

    /// Zotero keeps chapters and books with no title (43 of the owner's 2812
    /// works): `bibliographic_items.title` is NULLABLE. Such a work used to
    /// make the whole leg fail with "Invalid column type Null ... title".
    fn blank_the_title_of(conn: &Connection, item_key: &str) {
        conn.execute(
            "UPDATE bibliographic_items SET title = NULL WHERE item_key = ?1",
            [item_key],
        )
        .expect("blank the title");
    }

    #[test]
    fn leg_survives_a_work_without_a_title_and_still_cites_it() {
        let (conn, chunk_a, _) = two_library_catalog();
        blank_the_title_of(&conn, "KA0001");
        let leg = bibliography_leg(&conn, CONTRACT, "compartido", &[], &params(), &embed_ok);
        assert_eq!(leg.notice, None, "a titleless work is not a failure");
        assert_eq!(leg.detail, None);
        assert_eq!(leg.sources.len(), 2, "its sibling work is still found");
        let untitled = leg
            .sources
            .iter()
            .find(|s| s.bibliography.as_ref().unwrap().chunk_id == chunk_a)
            .expect("the titleless work's passage");
        assert!(
            untitled.item_title.contains("KA0001"),
            "the citation names the work by its key instead of showing nothing: {:?}",
            untitled.item_title
        );
    }

    #[test]
    fn work_search_survives_a_work_without_a_title() {
        let (conn, _, _) = two_library_catalog();
        blank_the_title_of(&conn, "KA0001");
        let answer = crate::bibliography::retrieval::search_works(
            &conn,
            CONTRACT,
            &crate::bibliography::retrieval::HybridQuery {
                text: "compartido".into(),
                top_k: 10,
                filters: WorkFilters::default(),
            },
            &embed_ok,
        )
        .expect("a titleless work must not fail the search");
        assert_eq!(answer.hits.len(), 2);
    }

    #[test]
    fn the_query_is_embedded_once_per_search() {
        let (conn, _, _) = two_library_catalog();
        let calls = std::cell::Cell::new(0u32);
        let leg = bibliography_leg(&conn, CONTRACT, "compartido", &[], &params(), &|text| {
            calls.set(calls.get() + 1);
            embed_ok(text)
        });
        assert_eq!(leg.notice, None);
        assert_eq!(calls.get(), 1, "one network call, not one per search stage");
    }

    #[test]
    fn a_failed_leg_says_what_failed_without_secrets() {
        let (conn, _, _) = two_library_catalog();
        conn.execute_batch("DROP TABLE bibliographic_chunk_spans")
            .unwrap();
        let leg = bibliography_leg(&conn, CONTRACT, "compartido", &[], &params(), &embed_ok);
        assert_eq!(leg.notice, Some(BibliographyNotice::Failed));
        let detail = leg.detail.expect("the failure carries its cause");
        assert!(detail.contains("bibliographic_chunk_spans"), "{detail}");
        assert!(detail.chars().count() <= 240, "short: {detail}");
    }

    #[test]
    fn passage_search_carries_the_failure_detail_too() {
        let (conn, _, _) = two_library_catalog();
        conn.execute_batch("DROP TABLE bibliographic_chunk_spans")
            .unwrap();
        let found = passage_search(&conn, CONTRACT, "compartido", &[], &params(), &embed_ok);
        assert_eq!(found.notice, Some(BibliographyNotice::Failed));
        assert!(found.detail.is_some());
    }

    #[test]
    fn leg_filters_by_library() {
        let (conn, _, chunk_b) = two_library_catalog();
        let only_b = [RagLibraryRef {
            library_type: "user".into(),
            library_id: "lib-b".into(),
        }];
        let leg = bibliography_leg(&conn, CONTRACT, "compartido", &only_b, &params(), &embed_ok);
        assert_eq!(leg.notice, None);
        let chunks: Vec<_> = leg
            .sources
            .iter()
            .map(|s| s.bibliography.as_ref().unwrap().chunk_id.clone())
            .collect();
        assert_eq!(chunks, [chunk_b]);
    }

    #[test]
    fn leg_reports_no_library_synced_for_unknown_or_missing_libraries() {
        let (conn, _, _) = two_library_catalog();
        let unknown = [RagLibraryRef {
            library_type: "group".into(),
            library_id: "404".into(),
        }];
        let leg = bibliography_leg(
            &conn,
            CONTRACT,
            "compartido",
            &unknown,
            &params(),
            &embed_ok,
        );
        assert_eq!(leg.notice, Some(BibliographyNotice::NoLibrarySynced));
        assert!(leg.sources.is_empty());

        let empty = passage_db();
        let leg = bibliography_leg(&empty, CONTRACT, "compartido", &[], &params(), &embed_ok);
        assert_eq!(leg.notice, Some(BibliographyNotice::NoLibrarySynced));
    }

    #[test]
    fn leg_reports_no_embeddings_without_running_the_embedder() {
        let mut conn = passage_db();
        let (_l, _item) = seed_item(&mut conn, "pa", "a", "KA0001", "Obra A");
        let leg = bibliography_leg(&conn, CONTRACT, "compartido", &[], &params(), &|_| {
            panic!("the embedder must not run without an active generation")
        });
        assert_eq!(leg.notice, Some(BibliographyNotice::NoEmbeddings));
        assert!(leg.sources.is_empty());
    }

    #[test]
    fn leg_reports_embedding_unavailable_when_the_query_cannot_be_embedded() {
        let (conn, _, _) = two_library_catalog();
        let leg = bibliography_leg(&conn, CONTRACT, "compartido", &[], &params(), &|_| {
            Err("sin clave".into())
        });
        assert_eq!(leg.notice, Some(BibliographyNotice::EmbeddingUnavailable));
        assert!(leg.sources.is_empty());
    }

    #[test]
    fn leg_applies_the_minimum_similarity_to_its_own_scale() {
        let (conn, chunk_a, _) = two_library_catalog();
        let mut strict = params();
        strict.min_similarity = 0.99;
        let leg = bibliography_leg(&conn, CONTRACT, "compartido", &[], &strict, &embed_ok);
        let chunks: Vec<_> = leg
            .sources
            .iter()
            .map(|s| s.bibliography.as_ref().unwrap().chunk_id.clone())
            .collect();
        assert_eq!(chunks, [chunk_a], "only the near-parallel passage survives");
    }

    #[test]
    fn leg_cites_html_snapshots_by_paragraph_never_by_page() {
        let (conn, chunk_a, _) = two_library_catalog();
        let page = "Uno.\n\nDos aquí.\n\nTres aquí.\n\nCuatro.";
        let start = page.find("Dos").unwrap();
        let end = page.find("Cuatro").unwrap() - 2;
        conn.execute(
            "UPDATE zotero_attachments SET content_type = 'text/html'
              WHERE id = ?1",
            [format!("att-{chunk_a}")],
        )
        .unwrap();
        conn.execute(
            "UPDATE bibliographic_chunk_spans SET start_char = ?2, end_char = ?3
              WHERE chunk_id = ?1",
            rusqlite::params![chunk_a, start as i64, end as i64],
        )
        .unwrap();
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0053_bibliographic_page_texts.sql"
        ))
        .unwrap();
        conn.execute(
            "INSERT INTO bibliographic_page_texts
               (attachment_id, page_number, method, text_content, text_hash,
                text_chars, quality, created_at, updated_at)
             VALUES (?1, 3, 'native', ?2, 'h', ?3, 'rich', 1, 1)",
            rusqlite::params![format!("att-{chunk_a}"), page, page.chars().count() as i64],
        )
        .unwrap();
        let leg = bibliography_leg(&conn, CONTRACT, "compartido", &[], &params(), &embed_ok);
        let located = leg
            .sources
            .iter()
            .find(|s| s.bibliography.as_ref().unwrap().chunk_id == chunk_a)
            .unwrap();
        assert_eq!(
            located.bibliography.as_ref().unwrap().location,
            Some(RagBibliographyLocation {
                kind: "paragraphs".into(),
                from: 2,
                to: 3
            })
        );
    }

    #[test]
    fn passage_search_lists_cards_with_csl_snippet_and_location() {
        let (conn, chunk_a, _) = two_library_catalog();
        let found = passage_search(&conn, CONTRACT, "compartido", &[], &params(), &embed_ok);
        assert_eq!(found.notice, None);
        assert_eq!(found.passages.len(), 2);
        let first = &found.passages[0];
        assert_eq!(first.chunk_id, chunk_a);
        assert_eq!(first.item_key, "KA0001");
        assert_eq!(first.title, "Obra A");
        assert_eq!(first.library_native_id, "lib-a");
        assert_eq!(first.snippet, "Texto de la obra A.");
        assert!(!first.csl_json.is_empty(), "a citation needs the CSL data");
        assert_eq!(
            first.location,
            Some(RagBibliographyLocation {
                kind: "pages".into(),
                from: 3,
                to: 3
            })
        );
    }

    #[test]
    fn passage_search_says_why_it_found_nothing() {
        let empty = passage_db();
        let found = passage_search(&empty, CONTRACT, "compartido", &[], &params(), &embed_ok);
        assert_eq!(found.notice, Some(BibliographyNotice::NoLibrarySynced));
        assert!(found.passages.is_empty());

        let mut conn = passage_db();
        let (_l, _item) = seed_item(&mut conn, "pa", "a", "KA0001", "Obra A");
        let found = passage_search(&conn, CONTRACT, "compartido", &[], &params(), &|_| {
            panic!("the embedder must not run without an active generation")
        });
        assert_eq!(found.notice, Some(BibliographyNotice::NoEmbeddings));
    }

    /// Harness against a COPY of a real archive (never the live file):
    /// `ENTROPIA_CHAT_DB=<copy.sqlite> cargo test owner_db_copy -- --ignored --nocapture`.
    /// The query embedder is a deterministic fake of the contract's width, so
    /// no network and no key are involved: what it exercises is every SQL and
    /// shape assumption of the three bibliography entry points.
    #[test]
    #[ignore = "needs ENTROPIA_CHAT_DB pointing at a copy of an archive"]
    fn owner_db_copy_bibliography_search_harness() {
        let path = std::env::var("ENTROPIA_CHAT_DB").expect("ENTROPIA_CHAT_DB");
        let conn = crate::db::open::open_archive_connection(std::path::Path::new(&path))
            .expect("open the copy");
        let contract = crate::processing::eligibility::resolve_effective_embedding_contract(&conn)
            .expect("effective contract");
        println!(
            "query contract: provider={} model={} dims={} hash={}",
            contract.provider, contract.model, contract.dimensions, contract.hash
        );
        let stored: Vec<(String, String)> = conn
            .prepare("SELECT contract_hash, status FROM bibliographic_index_generations")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        println!("stored generations: {stored:?}");

        let dims = contract.dimensions;
        let fake = move |text: &str| -> Result<Vec<f32>, String> {
            let seed = text
                .bytes()
                .fold(7u32, |acc, b| acc.wrapping_mul(31) ^ b as u32);
            Ok((0..dims)
                .map(|i| {
                    (((seed.wrapping_add((i as u32).wrapping_mul(2654435761))) % 2000) as f32
                        - 1000.0)
                        / 1000.0
                })
                .collect())
        };
        let question = "Necesito información sobre sindicalismo en Mar del Plata";
        let params = RagParams::default();

        let started = std::time::Instant::now();
        let leg = run_bibliography_leg(&conn, &contract.hash, question, &[], &params, &fake);
        println!(
            "run_bibliography_leg(all synced): {:?} in {:?}",
            leg.as_ref()
                .map(|leg| (leg.sources.len(), leg.notice))
                .map_err(|e| e.clone()),
            started.elapsed()
        );
        let one = [RagLibraryRef {
            library_type: "user".into(),
            library_id: "0".into(),
        }];
        let started = std::time::Instant::now();
        let leg = run_bibliography_leg(&conn, &contract.hash, question, &one, &params, &fake);
        println!(
            "run_bibliography_leg(user/0): {:?} in {:?}",
            leg.as_ref()
                .map(|leg| (leg.sources.len(), leg.notice))
                .map_err(|e| e.clone()),
            started.elapsed()
        );
        let works = crate::bibliography::retrieval::search_works(
            &conn,
            &contract.hash,
            &crate::bibliography::retrieval::HybridQuery {
                text: question.to_string(),
                top_k: 20,
                filters: WorkFilters::default(),
            },
            &fake,
        );
        println!(
            "search_works: {:?}",
            works
                .as_ref()
                .map(|a| (a.hits.len(), a.vector_available))
                .map_err(|e| format!("{}: {}", e.code, e.message))
        );
        let found = passage_search(&conn, &contract.hash, question, &[], &params, &fake);
        println!(
            "passage_search: passages={} notice={:?}",
            found.passages.len(),
            found.notice
        );
        for passage in found.passages.iter().take(3) {
            println!("  {} | {}", passage.title, passage.score);
        }
    }
}
