//! Hybrid work retrieval (E3c-WU3): lexical FTS5 over profile texts, vector
//! cosine over the active generation of the query contract, and catalog
//! filters — fused by rank, never by cross-space score comparison.
//!
//! Honesty rules, all enforced here and pinned by tests:
//! - The vector leg only reads vectors stamped with the ACTIVE generation
//!   of the query contract, with matching dimensions and finite values.
//! - A stored vector whose profile hash moved (metadata edited after
//!   publish) is excluded from semantic results even while its generation
//!   stays active for other works (plan section 314).
//! - Tombstoned works are ineligible on every leg.
//! - With no queryable space (no active generation, or the engine
//!   unavailable), the answer is lexical-only and says so: `vector_available
//!   == false`. It never pretends to be hybrid.
//! - Fusion is reciprocal-rank fusion over per-leg ranks: bm25 magnitudes
//!   and cosine similarities are never compared numerically across legs.
//! - Every hit carries its method, per-leg scores, contract, generation,
//!   and the filters that applied.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension as _};

use super::repository::BibliographyResult;

/// Catalog filters applied after candidate collection, before ranking.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkFilters {
    /// Internal `zotero_libraries.id` rows; empty means every library.
    pub library_ids: Vec<String>,
    /// Inclusive year bounds from the CSL `issued` date part.
    pub year_from: Option<i64>,
    pub year_to: Option<i64>,
    /// CSL work types (`book`, `journalArticle`, ...); empty means every type.
    pub item_types: Vec<String>,
    /// Zotero tags; a work matches when it carries ANY of them.
    pub tags: Vec<String>,
}

/// One hybrid query over the bibliography.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HybridQuery {
    pub text: String,
    pub top_k: usize,
    pub filters: WorkFilters,
}

/// One ranked work with full provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkHit {
    pub item_id: String,
    pub item_key: String,
    pub library_id: String,
    pub title: String,
    /// `lexical`, `vector`, or `hybrid` (present in both legs).
    pub method: String,
    /// bm25 rank score; `None` when the work came only from the vector leg.
    /// Lower (more negative) is better; never compared to cosine values.
    pub lexical_score: Option<f64>,
    /// Cosine similarity in [-1, 1]; `None` when lexical-only.
    pub vector_score: Option<f64>,
    /// RRF fused score used for ordering.
    pub fused_score: f64,
    /// Space identity of the vector leg; `None` on lexical-only answers.
    pub contract_hash: Option<String>,
    pub generation_id: Option<String>,
}

/// A hybrid answer: ranked hits plus what the answer claims to be.
#[derive(Debug, Clone, PartialEq)]
pub struct HybridAnswer {
    pub hits: Vec<WorkHit>,
    /// False when no queryable vector space existed: the answer is
    /// lexical-only and the UI must label it as such.
    pub vector_available: bool,
    pub active_generation_id: Option<String>,
    pub contract_hash: String,
}

/// One vector candidate: identity plus its stored bytes. The caller embeds
/// the query and fuses; this keeps engine access at the boundary.
#[derive(Debug, Clone)]
pub struct VectorCandidate {
    pub item_id: String,
    pub embedding: Vec<u8>,
    pub dimensions: i64,
    pub input_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bibliography::generation::{
        begin_index_generation, complete_index_generation, note_generation_progress,
        register_embedding_contract, set_generation_manifest, EmbeddingContractRow,
    };
    use crate::bibliography::profile::{
        build_profile, ProfileInput, BIBLIOGRAPHY_PROFILE_TEMPLATE_V1,
    };
    use crate::bibliography::repository::{
        upsert_connection, upsert_item, upsert_library, upsert_semantic_profile,
        BibliographicItemInput, LibraryType, SourceOrigin, UpsertConnection, UpsertLibrary,
    };

    const CONTRACT: &str = "contract-search";
    const FAKE_MODEL: &str = "fake/model";

    fn search_db() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0040_bibliography_catalog.sql"
        ))
        .expect("apply catalog foundation");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0041_bibliography_relations.sql"
        ))
        .expect("apply relations");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0047_bibliographic_semantic_profiles.sql"
        ))
        .expect("apply profiles table");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0049_bibliographic_index_generations.sql"
        ))
        .expect("apply generations");
        // Test-only stub of the pre-0050 embeddings shape (see 0048): the
        // retrieval fixture never builds processing tables, so the 0048
        // rebuild cannot run here. The real file stays pinned by the store
        // mirror tests.
        conn.execute_batch(
            "CREATE TABLE bibliographic_item_embeddings (
               item_id TEXT NOT NULL,
               embedding_contract TEXT NOT NULL,
               embedding_model TEXT NOT NULL,
               dimensions INTEGER NOT NULL,
               embedding BLOB NOT NULL,
               input_hash TEXT NOT NULL,
               profile_revision INTEGER NOT NULL,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (item_id, embedding_contract)
             );",
        )
        .expect("stub pre-0050 embeddings");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0050_bibliographic_embedding_generations.sql"
        ))
        .expect("apply embedding generations");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0051_bibliographic_profile_fts.sql"
        ))
        .expect("apply profile FTS");
        conn
    }

    pub(crate) fn contract_row() -> EmbeddingContractRow {
        EmbeddingContractRow {
            contract_hash: CONTRACT.to_string(),
            provider: "api".to_string(),
            model: FAKE_MODEL.to_string(),
            dimensions: 4,
            chunking_contract: "test-chunking".to_string(),
        }
    }

    fn seed_work(
        conn: &mut Connection,
        item_key: &str,
        title: &str,
        abstract_text: &str,
        year: i64,
        item_type: &str,
        tags: Vec<&str>,
    ) -> String {
        let source = upsert_connection(
            conn,
            UpsertConnection {
                id: format!("conn-{item_key}"),
                source_origin: SourceOrigin::Local,
                source_instance_id: None,
                endpoint: Some("http://synthetic.invalid".to_string()),
                capabilities_json: r#"{"read":true}"#.to_string(),
            },
        )
        .expect("connection");
        let library = upsert_library(
            conn,
            UpsertLibrary {
                connection_id: source.id,
                library_type: LibraryType::User,
                library_id: format!("lib-{item_key}"),
                name: format!("Personal {item_key}"),
                last_modified_version: Some(7),
            },
        )
        .expect("library");
        let csl = serde_json::json!({
            "id": item_key,
            "type": item_type,
            "title": title,
            "abstract": abstract_text,
            "publisher": "Editorial Universitaria",
            "issued": { "date-parts": [[year]] },
            "author": [{ "family": "Pérez", "given": "Ana" }],
        });
        let native = serde_json::json!({
            "key": item_key,
            "version": 3,
            "itemType": "book",
            "tags": tags.iter().map(|tag| serde_json::json!({ "tag": tag })).collect::<Vec<_>>(),
        });
        upsert_item(
            conn,
            &library.id,
            BibliographicItemInput {
                item_key: item_key.to_string(),
                item_version: Some(3),
                native_json_snapshot: native.to_string(),
                csl_json_snapshot: csl.to_string(),
                title: Some(title.to_string()),
                ..Default::default()
            },
        )
        .expect("catalog item")
        .id
    }

    #[test]
    fn zotero_library_resolves_to_the_internal_rows_it_names() {
        let mut conn = search_db();
        seed_work(&mut conn, "a", "Uno", "x", 2020, "book", vec![]);
        seed_work(&mut conn, "b", "Dos", "x", 2020, "book", vec![]);
        let rows_a = resolve_zotero_library_rows(&conn, "user", "lib-a").expect("resolve");
        let stored: String = conn
            .query_row(
                "SELECT id FROM zotero_libraries WHERE library_id = 'lib-a'",
                [],
                |row| row.get(0),
            )
            .expect("row");
        assert_eq!(rows_a, vec![stored]);
        // The type is part of the identity: a group with the same numeric id
        // is a different library, and an unknown one resolves to nothing.
        assert!(resolve_zotero_library_rows(&conn, "group", "lib-a")
            .expect("resolve")
            .is_empty());
        assert!(resolve_zotero_library_rows(&conn, "user", "nope")
            .expect("resolve")
            .is_empty());
    }

    #[test]
    fn work_display_names_authors_year_and_the_native_library() {
        let mut conn = search_db();
        let id = seed_work(&mut conn, "disp", "Obra", "x", 2021, "book", vec![]);
        let display = read_work_display(&conn, &id)
            .expect("read")
            .expect("work exists");
        assert_eq!(display.authors, "Pérez");
        assert_eq!(display.year, Some(2021));
        assert_eq!(display.library_name, "Personal disp");
        // The native identity the ficha opens by, not the internal row id.
        assert_eq!(display.library_type, "user");
        assert_eq!(display.library_native_id, "lib-disp");
        assert!(display.csl_json.contains("\"Obra\""));
        assert!(read_work_display(&conn, "missing").expect("read").is_none());
    }

    #[test]
    fn work_display_survives_csl_without_authors_or_date() {
        let mut conn = search_db();
        let id = seed_work(&mut conn, "bare", "Sin datos", "x", 2000, "book", vec![]);
        conn.execute(
            "UPDATE bibliographic_items SET csl_json_snapshot = ?1 WHERE id = ?2",
            rusqlite::params![
                r#"{"id":"bare","author":[{"literal":"Colectivo X"},{}]}"#,
                id
            ],
        )
        .expect("rewrite csl");
        let display = read_work_display(&conn, &id)
            .expect("read")
            .expect("work exists");
        assert_eq!(display.authors, "Colectivo X");
        assert_eq!(display.year, None);
    }

    fn profile_text(
        title: &str,
        abstract_text: &str,
        year: i64,
        item_type: &str,
        tags: &[&str],
    ) -> (String, String) {
        let input = ProfileInput {
            title: title.to_string(),
            creators: vec![("Pérez".to_string(), "Ana".to_string())],
            year: Some(year),
            item_type: item_type.to_string(),
            publication: "Editorial Universitaria".to_string(),
            abstract_text: abstract_text.to_string(),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
        };
        let built = build_profile(&input);
        (built.canonical_text, built.input_hash)
    }

    fn publish_profile(
        conn: &mut Connection,
        item_id: &str,
        canonical_text: &str,
        input_hash: &str,
    ) {
        let provenance = serde_json::json!([{ "field": "title", "line": "Título: x" }]).to_string();
        upsert_semantic_profile(
            conn,
            item_id,
            BIBLIOGRAPHY_PROFILE_TEMPLATE_V1,
            canonical_text,
            input_hash,
            &provenance,
            1_000,
        )
        .expect("publish profile");
    }

    fn publish_vector(
        conn: &Connection,
        item_id: &str,
        generation_id: &str,
        vector: &[f32],
        input_hash: &str,
    ) {
        assert_eq!(vector.len(), 4);
        let blob: Vec<u8> = vector
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        conn.execute(
            "INSERT INTO bibliographic_item_embeddings
               (item_id, generation_id, embedding_contract, embedding_model,
                dimensions, embedding, input_hash, profile_revision, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 4, ?5, ?6, 1, 1, 1)",
            rusqlite::params![
                item_id,
                generation_id,
                CONTRACT,
                FAKE_MODEL,
                blob,
                input_hash
            ],
        )
        .expect("publish vector");
    }

    fn activate_generation_with(conn: &mut Connection, generation_id: &str, expected: i64) {
        register_embedding_contract(conn, &contract_row(), 1).expect("register contract");
        if super::super::generation::active_generation(conn, CONTRACT)
            .expect("active read")
            .is_none()
        {
            let _ = begin_index_generation(conn, CONTRACT, generation_id, 10).expect("begin");
        }
        // Attach semantics: the begin above returns the existing staging row
        // when one is already open; resolve the real id for the manifest.
        let staging: String = conn
            .query_row(
                "SELECT id FROM bibliographic_index_generations
                 WHERE contract_hash = ?1 AND status = 'staging'",
                [CONTRACT],
                |row| row.get(0),
            )
            .expect("staging id");
        // Test-only accounting: the lifecycle itself is pinned in
        // generation.rs tests; here the generation just needs to be active.
        // A zero manifest never activates, so floor it at one entry.
        let floor = expected.max(1);
        set_generation_manifest(conn, &staging, floor, 11).expect("manifest");
        for _ in 0..floor {
            note_generation_progress(conn, &staging).expect("progress");
        }
        complete_index_generation(&mut *conn, &staging, 20).expect("activate");
    }

    #[test]
    fn lexical_leg_finds_the_matching_work() {
        let mut conn = search_db();
        let a = seed_work(
            &mut conn,
            "LEXA0001",
            "Revoluciones del siglo",
            "Asociaciones políticas.",
            2018,
            "book",
            vec!["siglo XIX"],
        );
        let b = seed_work(
            &mut conn,
            "LEXB0001",
            "Botánica tropical",
            "Helechos y musgos.",
            2020,
            "book",
            vec!["biología"],
        );
        let (text_a, hash_a) = profile_text(
            "Revoluciones del siglo",
            "Asociaciones políticas.",
            2018,
            "book",
            &["siglo XIX"],
        );
        let (text_b, hash_b) = profile_text(
            "Botánica tropical",
            "Helechos y musgos.",
            2020,
            "book",
            &["biología"],
        );
        publish_profile(&mut conn, &a, &text_a, &hash_a);
        publish_profile(&mut conn, &b, &text_b, &hash_b);

        let hits = lexical_candidates(&conn, "revoluciones", 10).expect("lexical");
        assert_eq!(hits.len(), 1, "only the matching work hits, got {hits:?}");
        assert_eq!(hits[0].0, a);

        let both = lexical_candidates(&conn, "Editorial Universitaria", 10).expect("lexical");
        assert_eq!(both.len(), 2, "shared publisher lines match both works");
    }

    #[test]
    fn lexical_leg_ignores_deleted_profiles() {
        let mut conn = search_db();
        let a = seed_work(
            &mut conn,
            "LEXD0001",
            "Obra efímera",
            "Texto único efímero.",
            2018,
            "book",
            vec![],
        );
        let (text_a, hash_a) =
            profile_text("Obra efímera", "Texto único efímero.", 2018, "book", &[]);
        publish_profile(&mut conn, &a, &text_a, &hash_a);
        assert_eq!(
            lexical_candidates(&conn, "efímera", 10)
                .expect("lexical")
                .len(),
            1
        );
        conn.execute(
            "DELETE FROM bibliographic_semantic_profiles WHERE item_id = ?1",
            [&a],
        )
        .expect("delete profile");
        assert!(
            lexical_candidates(&conn, "efímera", 10)
                .expect("lexical")
                .is_empty(),
            "a lexical hit must never describe a deleted profile"
        );
    }

    #[test]
    fn vector_leg_excludes_moved_inputs_and_foreign_spaces() {
        let mut conn = search_db();
        let a = seed_work(
            &mut conn,
            "VECA0001",
            "Obra A",
            "Texto A.",
            2018,
            "book",
            vec![],
        );
        let b = seed_work(
            &mut conn,
            "VECB0001",
            "Obra B",
            "Texto B.",
            2019,
            "book",
            vec![],
        );
        let (text_a, hash_a) = profile_text("Obra A", "Texto A.", 2018, "book", &[]);
        let (text_b, hash_b) = profile_text("Obra B", "Texto B.", 2019, "book", &[]);
        publish_profile(&mut conn, &a, &text_a, &hash_a);
        publish_profile(&mut conn, &b, &text_b, &hash_b);
        activate_generation_with(&mut conn, "gen-vec", 0);
        let active: String = conn
            .query_row(
                "SELECT id FROM bibliographic_index_generations
                 WHERE contract_hash = ?1 AND status = 'active'",
                [CONTRACT],
                |row| row.get(0),
            )
            .expect("active id");
        publish_vector(&conn, &a, &active, &[1.0, 0.0, 0.0, 0.0], &hash_a);
        publish_vector(&conn, &b, &active, &[0.0, 1.0, 0.0, 0.0], &hash_b);
        // A second contract with its own active generation must never leak
        // into this contract's candidate set.
        register_embedding_contract(
            &conn,
            &EmbeddingContractRow {
                contract_hash: "contract-other".to_string(),
                provider: "api".to_string(),
                model: "other/model".to_string(),
                dimensions: 4,
                chunking_contract: "test-chunking".to_string(),
            },
            1,
        )
        .expect("other contract");
        let other_gen =
            begin_index_generation(&conn, "contract-other", "gen-other", 10).expect("begin");
        set_generation_manifest(&conn, &other_gen.id, 1, 11).expect("manifest");
        note_generation_progress(&conn, &other_gen.id).expect("progress");
        complete_index_generation(&mut conn, &other_gen.id, 20).expect("activate");
        publish_vector(&conn, &b, "gen-other", &[1.0, 0.0, 0.0, 0.0], &hash_b);

        let candidates = vector_candidates_for_generation(&conn, &active).expect("candidates");
        let ids: Vec<&str> = candidates
            .iter()
            .map(|candidate| candidate.item_id.as_str())
            .collect();
        assert_eq!(
            ids.len(),
            2,
            "only this generation's rows surface, got {ids:?}"
        );
        assert!(ids.contains(&a.as_str()) && ids.contains(&b.as_str()));

        // Simulate a metadata edit after publish: the stored profile hash
        // moves while the vector keeps the old one.
        let (moved_text, moved_hash) =
            profile_text("Obra B", "Texto B corregido.", 2019, "book", &[]);
        assert_ne!(moved_hash, hash_b);
        publish_profile(&mut conn, &b, &moved_text, &moved_hash);
        let answer = search_works(
            &conn,
            CONTRACT,
            &HybridQuery {
                text: "Texto".to_string(),
                top_k: 10,
                filters: WorkFilters::default(),
            },
            &|_text| Ok(vec![0.0, 1.0, 0.0, 0.0]),
        )
        .expect("search");
        assert!(
            answer.vector_available,
            "the space stays queryable for the unchanged work"
        );
        let vector_hits: Vec<&WorkHit> = answer
            .hits
            .iter()
            .filter(|hit| hit.method != "lexical")
            .collect();
        assert_eq!(
            vector_hits.len(),
            1,
            "the moved work drops out, got {:?}",
            answer.hits
        );
        assert_eq!(vector_hits[0].item_id, a);
        assert_eq!(vector_hits[0].contract_hash.as_deref(), Some(CONTRACT));
        assert_eq!(
            vector_hits[0].generation_id.as_deref(),
            Some(active.as_str())
        );
    }

    #[test]
    fn fusion_combines_ranks_without_comparing_scores() {
        let lexical = vec![
            ("a".to_string(), -10.0),
            ("b".to_string(), -1.0),
            ("c".to_string(), -0.5),
        ];
        let vector = vec![("c".to_string(), 0.99)];
        let fused = reciprocal_rank_fusion(&lexical, &vector, 10);
        let order: Vec<&str> = fused.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(
            order,
            vec!["c", "a", "b"],
            "the work both legs agree on wins without comparing -10.0 to 0.99"
        );
        let c_score = fused.iter().find(|(id, _)| id == "c").expect("c fused").1;
        let expected = 1.0 / 62.0 + 1.0 / 60.0;
        assert!(
            (c_score - expected).abs() < 1e-12,
            "RRF with k=60: lex rank 2 + vec rank 0, got {c_score}"
        );
    }

    #[test]
    fn search_applies_filters_and_labels_provenance() {
        let mut conn = search_db();
        let a = seed_work(
            &mut conn,
            "FILTA001",
            "Historia común",
            "Pasado compartido.",
            2018,
            "book",
            vec!["historia"],
        );
        let b = seed_work(
            &mut conn,
            "FILTB001",
            "Ciencia común",
            "Futuro compartido.",
            2021,
            "journalArticle",
            vec!["ciencia"],
        );
        let (text_a, hash_a) = profile_text(
            "Historia común",
            "Pasado compartido.",
            2018,
            "book",
            &["historia"],
        );
        let (text_b, hash_b) = profile_text(
            "Ciencia común",
            "Futuro compartido.",
            2021,
            "journalArticle",
            &["ciencia"],
        );
        publish_profile(&mut conn, &a, &text_a, &hash_a);
        publish_profile(&mut conn, &b, &text_b, &hash_b);
        activate_generation_with(&mut conn, "gen-filter", 0);
        let active: String = conn
            .query_row(
                "SELECT id FROM bibliographic_index_generations
                 WHERE contract_hash = ?1 AND status = 'active'",
                [CONTRACT],
                |row| row.get(0),
            )
            .expect("active id");
        publish_vector(&conn, &a, &active, &[1.0, 0.0, 0.0, 0.0], &hash_a);
        publish_vector(&conn, &b, &active, &[1.0, 0.0, 0.0, 0.0], &hash_b);

        // Year filter keeps only the recent work; the method and provenance
        // still name both legs.
        let answer = search_works(
            &conn,
            CONTRACT,
            &HybridQuery {
                text: "común".to_string(),
                top_k: 10,
                filters: WorkFilters {
                    year_from: Some(2020),
                    ..Default::default()
                },
            },
            &|_text| Ok(vec![1.0, 0.0, 0.0, 0.0]),
        )
        .expect("search");
        assert_eq!(answer.hits.len(), 1, "the year filter narrows to one work");
        let hit = &answer.hits[0];
        assert_eq!(hit.item_id, b);
        assert_eq!(hit.method, "hybrid");
        assert!(hit.lexical_score.is_some() && hit.vector_score.is_some());
        assert_eq!(hit.contract_hash.as_deref(), Some(CONTRACT));
        assert_eq!(hit.generation_id.as_deref(), Some(active.as_str()));

        // Tag filter keeps only the matching work.
        let tagged = search_works(
            &conn,
            CONTRACT,
            &HybridQuery {
                text: "común".to_string(),
                top_k: 10,
                filters: WorkFilters {
                    tags: vec!["historia".to_string()],
                    ..Default::default()
                },
            },
            &|_text| Ok(vec![1.0, 0.0, 0.0, 0.0]),
        )
        .expect("search");
        assert_eq!(tagged.hits.len(), 1);
        assert_eq!(tagged.hits[0].item_id, a);

        // Type filter keeps only journal articles.
        let typed = search_works(
            &conn,
            CONTRACT,
            &HybridQuery {
                text: "común".to_string(),
                top_k: 10,
                filters: WorkFilters {
                    item_types: vec!["journalArticle".to_string()],
                    ..Default::default()
                },
            },
            &|_text| Ok(vec![1.0, 0.0, 0.0, 0.0]),
        )
        .expect("search");
        assert_eq!(typed.hits.len(), 1);
        assert_eq!(typed.hits[0].item_id, b);
    }

    #[test]
    fn search_without_an_active_generation_is_labeled_lexical_only() {
        let mut conn = search_db();
        let a = seed_work(
            &mut conn,
            "LEXO0001",
            "Obra léxica",
            "Vocabulario distintivo.",
            2018,
            "book",
            vec![],
        );
        let (text_a, hash_a) =
            profile_text("Obra léxica", "Vocabulario distintivo.", 2018, "book", &[]);
        publish_profile(&mut conn, &a, &text_a, &hash_a);

        let answer = search_works(
            &conn,
            CONTRACT,
            &HybridQuery {
                text: "distintivo".to_string(),
                top_k: 10,
                filters: WorkFilters::default(),
            },
            &|_text| panic!("the embedder must never run without a queryable space"),
        )
        .expect("search");
        assert!(!answer.vector_available, "no space means no vector claims");
        assert!(answer.active_generation_id.is_none());
        assert_eq!(answer.contract_hash, CONTRACT);
        assert_eq!(answer.hits.len(), 1, "lexical still finds the work");
        assert_eq!(answer.hits[0].method, "lexical");
        assert!(answer.hits[0].vector_score.is_none());
        assert!(answer.hits[0].contract_hash.is_none());
    }
}
/// FTS5 match bound: lexical candidates stay a bounded pre-filter, never
/// the whole catalog.
fn lexical_limit(top_k: usize) -> usize {
    top_k.saturating_mul(5).clamp(20, 500)
}

/// Lexical candidates over profile texts: FTS5 bm25 order, joined back to
/// the live profile row and excluding tombstoned works, so a hit can never
/// describe a deleted profile or an unlinked work.
pub fn lexical_candidates(
    conn: &Connection,
    query_text: &str,
    limit: usize,
) -> BibliographyResult<Vec<(String, f64)>> {
    let sanitized = crate::nlp::fts::sanitize_fts5_query(query_text);
    if sanitized.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(
            "SELECT bibliographic_profile_fts.item_id, bm25(bibliographic_profile_fts)
             FROM bibliographic_profile_fts
             JOIN bibliographic_semantic_profiles p ON p.item_id = bibliographic_profile_fts.item_id
             JOIN bibliographic_items i ON i.id = p.item_id
             LEFT JOIN zotero_item_tombstones t ON t.item_id = i.id
             WHERE bibliographic_profile_fts MATCH ?1 AND t.item_id IS NULL
             ORDER BY bm25(bibliographic_profile_fts) LIMIT ?2",
        )
        .map_err(|error| {
            crate::bibliography::repository::BibliographyError::new(
                "sql_error",
                format!("Failed to prepare lexical search: {error}"),
            )
        })?;
    let rows = stmt
        .query_map(rusqlite::params![sanitized, limit.max(1) as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })
        .map_err(|error| {
            crate::bibliography::repository::BibliographyError::new(
                "sql_error",
                format!("Failed to run lexical search: {error}"),
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            crate::bibliography::repository::BibliographyError::new(
                "sql_error",
                format!("Failed to read lexical search: {error}"),
            )
        })?;
    Ok(rows)
}

/// Vector candidates stamped with one generation: raw bytes plus the
/// stored input hash, so the caller can exclude moved inputs without
/// re-reading rows. Tombstoned works never surface.
pub fn vector_candidates_for_generation(
    conn: &Connection,
    generation_id: &str,
) -> BibliographyResult<Vec<VectorCandidate>> {
    let mut stmt = conn
        .prepare(
            "SELECT e.item_id, e.embedding, e.dimensions, e.input_hash
             FROM bibliographic_item_embeddings e
             JOIN bibliographic_items i ON i.id = e.item_id
             LEFT JOIN zotero_item_tombstones t ON t.item_id = e.item_id
             WHERE e.generation_id = ?1 AND t.item_id IS NULL",
        )
        .map_err(|error| {
            crate::bibliography::repository::BibliographyError::new(
                "sql_error",
                format!("Failed to prepare vector candidates: {error}"),
            )
        })?;
    let rows = stmt
        .query_map([generation_id], |row| {
            Ok(VectorCandidate {
                item_id: row.get(0)?,
                embedding: row.get(1)?,
                dimensions: row.get(2)?,
                input_hash: row.get(3)?,
            })
        })
        .map_err(|error| {
            crate::bibliography::repository::BibliographyError::new(
                "sql_error",
                format!("Failed to read vector candidates: {error}"),
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            crate::bibliography::repository::BibliographyError::new(
                "sql_error",
                format!("Failed to collect vector candidates: {error}"),
            )
        })?;
    Ok(rows)
}

/// RRF smoothing constant: each leg contributes `1 / (k + rank)`.
const RRF_K: f64 = 60.0;

/// Reciprocal-rank fusion over per-leg rank positions (0-based). Scores
/// from different legs are never compared numerically — only ranks fuse.
/// Ties break by item id for determinism.
pub fn reciprocal_rank_fusion(
    lexical: &[(String, f64)],
    vector: &[(String, f64)],
    top_k: usize,
) -> Vec<(String, f64)> {
    use std::collections::HashMap;
    let mut fused: HashMap<&str, f64> = HashMap::new();
    for (rank, (item_id, _)) in lexical.iter().enumerate() {
        *fused.entry(item_id.as_str()).or_default() += 1.0 / (RRF_K + rank as f64);
    }
    for (rank, (item_id, _)) in vector.iter().enumerate() {
        *fused.entry(item_id.as_str()).or_default() += 1.0 / (RRF_K + rank as f64);
    }
    let mut fused: Vec<(String, f64)> = fused
        .into_iter()
        .map(|(item_id, score)| (item_id.to_string(), score))
        .collect();
    fused.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    fused.truncate(top_k.max(1));
    fused
}

fn err(
    context: &str,
    error: impl std::fmt::Display,
) -> crate::bibliography::repository::BibliographyError {
    crate::bibliography::repository::BibliographyError::new(
        "sql_error",
        format!("{context}: {error}"),
    )
}

fn csl_year(value: &serde_json::Value) -> Option<i64> {
    value
        .get("issued")?
        .get("date-parts")?
        .as_array()?
        .first()?
        .as_array()?
        .first()?
        .as_i64()
}

fn native_tags(value: &serde_json::Value) -> Vec<String> {
    match value.get("tags").and_then(|value| value.as_array()) {
        Some(tags) => tags
            .iter()
            .filter_map(|tag| match tag {
                serde_json::Value::String(text) => Some(text.clone()),
                other => other
                    .get("tag")
                    .and_then(|text| text.as_str())
                    .map(String::from),
            })
            .collect(),
        None => Vec::new(),
    }
}

struct WorkMeta {
    item_key: String,
    library_id: String,
    title: String,
    year: Option<i64>,
    item_type: String,
    tags: Vec<String>,
}

fn read_work_meta(conn: &Connection, item_id: &str) -> BibliographyResult<Option<WorkMeta>> {
    // `title` is nullable in the catalog (Zotero keeps untitled chapters and
    // books): reading it as a plain String failed the whole search on the
    // first such work.
    let row: Option<(String, String, Option<String>, String, String)> = conn
        .query_row(
            "SELECT i.item_key, i.library_id, i.title, i.csl_json_snapshot, i.native_json_snapshot
             FROM bibliographic_items i
             LEFT JOIN zotero_item_tombstones t ON t.item_id = i.id
             WHERE i.id = ?1 AND t.item_id IS NULL",
            [item_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| err("Failed to read work metadata", error))?;
    let Some((item_key, library_id, title, csl_json, native_json)) = row else {
        return Ok(None);
    };
    let title = match title.filter(|title| !title.trim().is_empty()) {
        Some(title) => title,
        None => format!("Sin título ({item_key})"),
    };
    let csl: serde_json::Value = serde_json::from_str(&csl_json).unwrap_or(serde_json::Value::Null);
    let native: serde_json::Value =
        serde_json::from_str(&native_json).unwrap_or(serde_json::Value::Null);
    let csl_type = csl
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(Some(WorkMeta {
        item_key,
        library_id,
        title,
        year: csl_year(&csl),
        item_type: csl_type,
        tags: native_tags(&native),
    }))
}

fn matches_filters(meta: &WorkMeta, filters: &WorkFilters) -> bool {
    if !filters.library_ids.is_empty() && !filters.library_ids.contains(&meta.library_id) {
        return false;
    }
    if let Some(from) = filters.year_from {
        if meta.year.is_none_or(|year| year < from) {
            return false;
        }
    }
    if let Some(to) = filters.year_to {
        if meta.year.is_none_or(|year| year > to) {
            return false;
        }
    }
    if !filters.item_types.is_empty() && !filters.item_types.contains(&meta.item_type) {
        return false;
    }
    if !filters.tags.is_empty() && !filters.tags.iter().any(|tag| meta.tags.contains(tag)) {
        return false;
    }
    true
}

/// Maps a Zotero library as the UI names it (`library_type` + native
/// `library_id`, e.g. `user`/`0`) to the internal `zotero_libraries.id` rows
/// that [`WorkFilters::library_ids`] and every [`WorkHit::library_id`] use.
/// Empty means the library was never synced into the catalog. The type is
/// part of the identity: user 0 and group 0 are different libraries.
pub fn resolve_zotero_library_rows(
    conn: &Connection,
    library_type: &str,
    library_id: &str,
) -> BibliographyResult<Vec<String>> {
    let mut statement = conn
        .prepare(
            "SELECT id FROM zotero_libraries
              WHERE library_type = ?1 AND library_id = ?2
              ORDER BY id",
        )
        .map_err(|error| err("Failed to prepare Zotero library lookup", error))?;
    let rows = statement
        .query_map([library_type, library_id], |row| row.get::<_, String>(0))
        .map_err(|error| err("Failed to look up Zotero library", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| err("Failed to read Zotero library rows", error))?;
    Ok(rows)
}

/// What a result row and the ficha need to show one catalog work: the
/// reading metadata plus the native Zotero identity of its library (the
/// internal `zotero_libraries.id` of a [`WorkHit`] names nothing the UI can
/// open).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkDisplay {
    /// CSL authors' family names (or literal names), comma-separated.
    pub authors: String,
    pub year: Option<i64>,
    pub library_name: String,
    /// `user` or `group`, with the native id: the identity Zotero uses.
    pub library_type: String,
    pub library_native_id: String,
    /// The catalog's last CSL-JSON, what the ficha falls back to offline.
    pub csl_json: String,
}

fn csl_authors(value: &serde_json::Value) -> String {
    value
        .get("author")
        .and_then(|authors| authors.as_array())
        .map(|authors| {
            authors
                .iter()
                .filter_map(|author| {
                    let name = author
                        .get("family")
                        .or_else(|| author.get("literal"))
                        .and_then(|name| name.as_str())?
                        .trim();
                    (!name.is_empty()).then(|| name.to_string())
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// Reads the display data of one work by its internal item id. `None` when
/// the work (or its library) is gone.
pub fn read_work_display(
    conn: &Connection,
    item_id: &str,
) -> BibliographyResult<Option<WorkDisplay>> {
    let row: Option<(String, String, String, String)> = conn
        .query_row(
            "SELECT l.name, l.library_type, l.library_id, i.csl_json_snapshot
               FROM bibliographic_items i
               JOIN zotero_libraries l ON l.id = i.library_id
              WHERE i.id = ?1",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| err("Failed to read work display data", error))?;
    let Some((library_name, library_type, library_native_id, csl_json)) = row else {
        return Ok(None);
    };
    let csl: serde_json::Value = serde_json::from_str(&csl_json).unwrap_or(serde_json::Value::Null);
    Ok(Some(WorkDisplay {
        authors: csl_authors(&csl),
        year: csl_year(&csl),
        library_name,
        library_type,
        library_native_id,
        csl_json,
    }))
}

/// Full hybrid search: resolves the active generation of the query
/// contract, collects both legs, excludes moved inputs and tombstoned
/// works, applies filters, fuses, and annotates provenance. `embed_query`
/// computes the query vector; it is only called when a queryable space
/// exists.
pub fn search_works(
    conn: &Connection,
    contract_hash: &str,
    query: &HybridQuery,
    embed_query: &dyn Fn(&str) -> Result<Vec<f32>, String>,
) -> BibliographyResult<HybridAnswer> {
    let answer_for =
        |hits: Vec<WorkHit>, vector_available: bool, active_generation_id: Option<String>| {
            HybridAnswer {
                hits,
                vector_available,
                active_generation_id,
                contract_hash: contract_hash.to_string(),
            }
        };
    if query.text.trim().is_empty() {
        let active = crate::bibliography::generation::active_generation(conn, contract_hash)?;
        return Ok(answer_for(
            Vec::new(),
            active.is_some(),
            active.map(|generation| generation.id),
        ));
    }
    // Candidate identities from each leg, before metadata reads.
    let lexical = lexical_candidates(conn, &query.text, lexical_limit(query.top_k))?;
    // The vector leg only runs against the active generation of the query
    // contract. Anything else — no active row, an embed failure — falls
    // back to a labeled lexical-only answer.
    let active = crate::bibliography::generation::active_generation(conn, contract_hash)?;
    let Some(active) = active else {
        let hits = hits_for_lexical(conn, &lexical, &query.filters, query.top_k)?;
        return Ok(answer_for(hits, false, None));
    };
    let query_vector = match embed_query(&query.text) {
        Ok(vector) => vector,
        Err(_) => {
            let hits = hits_for_lexical(conn, &lexical, &query.filters, query.top_k)?;
            return Ok(answer_for(hits, false, Some(active.id)));
        }
    };
    if !query_vector.iter().all(|value| value.is_finite()) {
        let hits = hits_for_lexical(conn, &lexical, &query.filters, query.top_k)?;
        return Ok(answer_for(hits, false, Some(active.id)));
    }

    // Score the vector leg: same-generation rows only, matching dimensions,
    // finite values, fresh profile hashes, eligible works.
    let mut vector_scored: Vec<(String, f64)> = Vec::new();
    let mut vector_by_id: HashMap<String, f64> = HashMap::new();
    for candidate in vector_candidates_for_generation(conn, &active.id)? {
        if candidate.dimensions as usize != query_vector.len() {
            continue;
        }
        let stored = match crate::nlp::vector::decode_embedding_blob(&candidate.embedding) {
            Ok(stored) => stored,
            Err(_) => continue,
        };
        if !stored.iter().all(|value| value.is_finite()) {
            continue;
        }
        let distance = match crate::nlp::vector::cosine_distance(&query_vector, &stored) {
            Some(distance) => distance,
            None => continue,
        };
        // Plan section 314: a vector computed from older text is excluded
        // even while its generation stays active for other works.
        let fresh: Option<String> = conn
            .query_row(
                "SELECT input_hash FROM bibliographic_semantic_profiles WHERE item_id = ?1",
                [&candidate.item_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| err("Failed to read profile hash", error))?;
        if fresh.as_deref() != Some(candidate.input_hash.as_str()) {
            continue;
        }
        let similarity = 1.0 - distance;
        vector_by_id.insert(candidate.item_id.clone(), similarity);
        vector_scored.push((candidate.item_id, similarity));
    }
    vector_scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    // Filters apply to both legs before fusion, so a filtered-out rank
    // never donates its position.
    let mut lexical_filtered: Vec<(String, f64)> = Vec::new();
    let mut lexical_by_id: HashMap<String, f64> = HashMap::new();
    for (item_id, score) in &lexical {
        let Some(meta) = read_work_meta(conn, item_id)? else {
            continue;
        };
        if !matches_filters(&meta, &query.filters) {
            continue;
        }
        lexical_by_id.insert(item_id.clone(), *score);
        lexical_filtered.push((item_id.clone(), *score));
    }
    let mut vector_filtered: Vec<(String, f64)> = Vec::new();
    for (item_id, score) in &vector_scored {
        let Some(meta) = read_work_meta(conn, item_id)? else {
            continue;
        };
        if !matches_filters(&meta, &query.filters) {
            continue;
        }
        vector_filtered.push((item_id.clone(), *score));
    }

    let fused = reciprocal_rank_fusion(&lexical_filtered, &vector_filtered, query.top_k);
    let mut hits = Vec::with_capacity(fused.len());
    for (item_id, fused_score) in fused {
        let Some(meta) = read_work_meta(conn, &item_id)? else {
            continue;
        };
        let lexical_score = lexical_by_id.get(&item_id).copied();
        let vector_score = vector_by_id.get(&item_id).copied();
        let method = match (lexical_score, vector_score) {
            (Some(_), Some(_)) => "hybrid",
            (Some(_), None) => "lexical",
            (None, Some(_)) => "vector",
            (None, None) => continue,
        }
        .to_string();
        let (contract, generation) = if vector_score.is_some() {
            (Some(active.contract_hash.clone()), Some(active.id.clone()))
        } else {
            (None, None)
        };
        hits.push(WorkHit {
            item_id: item_id.clone(),
            item_key: meta.item_key,
            library_id: meta.library_id,
            title: meta.title,
            method,
            lexical_score,
            vector_score,
            fused_score,
            contract_hash: contract,
            generation_id: generation,
        });
    }
    // Fused order is rank order; the metadata re-read above cannot reorder.
    Ok(answer_for(hits, true, Some(active.id)))
}

/// Lexical-only hits with provenance and no vector claims.
fn hits_for_lexical(
    conn: &Connection,
    lexical: &[(String, f64)],
    filters: &WorkFilters,
    top_k: usize,
) -> BibliographyResult<Vec<WorkHit>> {
    let mut hits = Vec::new();
    for (item_id, score) in lexical.iter().take(top_k.max(1)) {
        let Some(meta) = read_work_meta(conn, item_id)? else {
            continue;
        };
        if !matches_filters(&meta, filters) {
            continue;
        }
        hits.push(WorkHit {
            item_id: item_id.clone(),
            item_key: meta.item_key,
            library_id: meta.library_id,
            title: meta.title,
            method: "lexical".to_string(),
            lexical_score: Some(*score),
            vector_score: None,
            fused_score: *score,
            contract_hash: None,
            generation_id: None,
        });
    }
    Ok(hits)
}

// ── Passage search ─────────────────────────────────────────────────────────

/// Why a passage is in the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassageMatchKind {
    /// Its text carries the words as typed (accents and case aside).
    Exact,
    /// Its text carries a close variant of them (an OCR misreading, a typo
    /// the spelling was corrected from): see [`crate::nlp::fuzzy`].
    Approximate,
    /// Only its vector was near the query's.
    Meaning,
}

impl PassageMatchKind {
    /// Stable machine string the frontend turns into words.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Approximate => "approximate",
            Self::Meaning => "meaning",
        }
    }
}

/// One ranked passage: a chunk with its work identity, spans, and the scores
/// of the legs that admitted it.
#[derive(Debug, Clone, PartialEq)]
pub struct PassageHit {
    pub chunk_id: String,
    pub item_id: String,
    pub item_key: String,
    pub library_id: String,
    pub title: String,
    pub attachment_id: String,
    pub ordinal: i64,
    pub text: String,
    pub spans: Vec<(i64, i64, i64)>,
    /// Cosine similarity between the chunk vector and the query; `0.0` when
    /// the query could not be embedded (a lexical-only answer).
    pub vector_score: f64,
    /// Why it is here: the lexical leg's reason when the text matched,
    /// otherwise `Meaning`.
    pub match_kind: PassageMatchKind,
    /// The words behind an exact or approximate match; empty for `Meaning`.
    pub match_terms: Vec<String>,
    /// Rank-fusion score of the two legs; only meaningful for ordering.
    pub fused_score: f64,
    pub generation_id: String,
    pub contract_hash: String,
}

/// Candidates each leg hands to the fusion. Deep enough that a per-work cap
/// or a filter never starves the answer, shallow enough to stay cheap.
const PASSAGE_LEG_DEPTH: usize = 200;

/// Words that carry no weight in a lexical match. They still count inside an
/// exact phrase.
const MATCH_STOPWORDS: &[&str] = &[
    "a", "al", "con", "de", "del", "e", "el", "en", "la", "las", "lo", "los", "o", "para", "por",
    "que", "se", "su", "sus", "u", "un", "una", "y", "an", "and", "as", "at", "by", "for", "in",
    "is", "of", "on", "or", "the", "to",
];

/// Lower-cases, folds accents and typographic ligatures (PDF text carries
/// "ﬁ") and collapses everything that is not a letter or digit into single
/// spaces. The result starts and ends with a space so a word-bounded phrase
/// is a plain substring. The accent table is the one the corpus search uses
/// ([`crate::nlp::fuzzy::normalize_term`]) so both agree on what a word is.
fn fold_for_match(text: &str, out: &mut String) {
    out.clear();
    out.push(' ');
    for ch in text.chars() {
        if ch.is_ascii() {
            if ch.is_ascii_alphanumeric() {
                out.push(ch.to_ascii_lowercase());
            } else if !out.ends_with(' ') {
                out.push(' ');
            }
            continue;
        }
        for lower in ch.to_lowercase() {
            let folded: &str = match lower {
                'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => "a",
                'é' | 'è' | 'ê' | 'ë' => "e",
                'í' | 'ì' | 'î' | 'ï' => "i",
                'ó' | 'ò' | 'ô' | 'ö' | 'õ' => "o",
                'ú' | 'ù' | 'û' | 'ü' => "u",
                'ñ' => "n",
                'ç' => "c",
                'ý' | 'ÿ' => "y",
                'ﬀ' => "ff",
                'ﬁ' => "fi",
                'ﬂ' => "fl",
                'ﬃ' => "ffi",
                'ﬄ' => "ffl",
                other if other.is_alphanumeric() => {
                    out.push(other);
                    continue;
                }
                _ => {
                    if !out.ends_with(' ') {
                        out.push(' ');
                    }
                    continue;
                }
            };
            out.push_str(folded);
        }
    }
    if !out.ends_with(' ') {
        out.push(' ');
    }
}

/// Rank key of a lexical match: exact phrase, distinct terms present, density.
type MatchRank = (bool, usize, f64);

/// A chunk's reason to be in the lexical leg.
struct LexicalCandidate {
    chunk_id: String,
    kind: PassageMatchKind,
    terms: Vec<String>,
}

/// A query prepared for the lexical leg.
struct LexicalQuery {
    /// Whole folded query, word-bounded.
    phrase: String,
    /// Distinct content words, folded.
    words: Vec<String>,
    /// The same words with a leading space, so each matches the start of a
    /// word ("casa" reaches "casas", never "ocasa").
    terms: Vec<String>,
    /// Terms a chunk must carry to count without the exact phrase.
    required: usize,
    /// Close variants of the words from the corpus vocabulary, folded.
    variants: Vec<String>,
    /// The variants word-bounded (` v `), the way they are searched.
    variant_terms: Vec<String>,
}

impl LexicalQuery {
    fn new(text: &str) -> Option<Self> {
        let mut phrase = String::new();
        fold_for_match(text, &mut phrase);
        if phrase.trim().is_empty() {
            return None;
        }
        let mut words: Vec<String> = Vec::new();
        for word in phrase.split_whitespace() {
            if !MATCH_STOPWORDS.contains(&word) && !words.iter().any(|known| known == word) {
                words.push(word.to_string());
            }
        }
        let terms = words.iter().map(|word| format!(" {word}")).collect();
        let count = words.len();
        let required = if count <= 2 {
            count
        } else {
            (count * 3).div_ceil(4)
        };
        Some(Self {
            phrase,
            words,
            terms,
            required,
            variants: Vec::new(),
            variant_terms: Vec::new(),
        })
    }

    /// Searches close variants of the words too, picked from `vocabulary`
    /// the way the corpus search picks them.
    fn with_variants(mut self, vocabulary: &HashMap<String, i64>) -> Self {
        for word in &self.words {
            for variant in crate::nlp::fuzzy::pick_variants(word, vocabulary) {
                if !self.words.contains(&variant) && !self.variants.contains(&variant) {
                    self.variants.push(variant);
                }
            }
        }
        self.variant_terms = self
            .variants
            .iter()
            .map(|variant| format!(" {variant} "))
            .collect();
        self
    }

    /// How the chunk matches, and its rank key (phrase, distinct terms or
    /// variants present, density; bigger is better); `None` when it does not.
    fn score(&self, folded: &str) -> Option<(PassageMatchKind, Vec<String>, MatchRank)> {
        let phrase = folded.contains(self.phrase.as_str());
        let mut present: Vec<&str> = Vec::new();
        let mut occurrences = 0usize;
        for (word, term) in self.words.iter().zip(&self.terms) {
            let found = folded.matches(term.as_str()).count();
            if found > 0 {
                present.push(word);
                occurrences += found;
            }
        }
        let density = |occurrences: usize| occurrences as f64 / folded.len().max(1) as f64;
        if phrase || (self.required > 0 && present.len() >= self.required) {
            let terms = if present.is_empty() {
                vec![self.phrase.trim().to_string()]
            } else {
                present.iter().map(|word| word.to_string()).collect()
            };
            return Some((
                PassageMatchKind::Exact,
                terms,
                (phrase, present.len(), density(occurrences)),
            ));
        }
        let mut found_variants: Vec<String> = Vec::new();
        let mut variant_occurrences = 0usize;
        for (variant, term) in self.variants.iter().zip(&self.variant_terms) {
            let found = folded.matches(term.as_str()).count();
            if found > 0 {
                found_variants.push(variant.clone());
                variant_occurrences += found;
            }
        }
        if found_variants.is_empty() {
            return None;
        }
        let count = found_variants.len();
        Some((
            PassageMatchKind::Approximate,
            found_variants,
            (false, count, density(variant_occurrences)),
        ))
    }
}

/// Works the filters admit; `None` when there is no filter at all.
fn eligible_items(
    conn: &Connection,
    filters: &WorkFilters,
) -> BibliographyResult<Option<HashSet<String>>> {
    if filters == &WorkFilters::default() {
        return Ok(None);
    }
    let mut stmt = conn
        .prepare("SELECT id, library_id FROM bibliographic_items")
        .map_err(|error| err("Failed to prepare eligible works", error))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| err("Failed to read eligible works", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| err("Failed to collect eligible works", error))?;
    drop(stmt);
    let only_library = filters.year_from.is_none()
        && filters.year_to.is_none()
        && filters.item_types.is_empty()
        && filters.tags.is_empty();
    let mut allowed = HashSet::new();
    for (item_id, library_id) in rows {
        if !filters.library_ids.is_empty() && !filters.library_ids.contains(&library_id) {
            continue;
        }
        if !only_library {
            let Some(meta) = read_work_meta(conn, &item_id)? else {
                continue;
            };
            if !matches_filters(&meta, filters) {
                continue;
            }
        }
        allowed.insert(item_id);
    }
    Ok(Some(allowed))
}

/// Chunks of the active generation, text only. The chunk row IS the current
/// text, so freshness of the vector is no concern of a text match; the
/// `EXISTS` is answered from the generation index without reading a vector.
const ACTIVE_CHUNK_TEXT_SQL: &str = "SELECT c.id, c.item_id, c.text_content
     FROM bibliographic_chunks c
     LEFT JOIN zotero_item_tombstones t ON t.item_id = c.item_id
     WHERE t.item_id IS NULL
       AND EXISTS (SELECT 1 FROM bibliographic_chunk_embeddings e
                    WHERE e.chunk_id = c.id AND e.generation_id = ?1)";

/// What the lexical leg searches: for every word of the active generation's
/// chunks, the chunks that hold it. Chunks have no FTS table (the corpus has
/// `fts_items` and its vocabulary view), and scanning 70 MB of text per query
/// costs seconds, so the index is built once by one scan of the chunk text
/// and kept in memory per archive and generation. It only nominates
/// candidates: each one is re-read from the database and judged on its real
/// text, so a stale entry can miss a new chunk but never invent a match.
/// The same words, with how many chunks hold each, are the vocabulary that
/// approximate matching picks variants from, as the corpus does with
/// `fts_items_vocab`.
struct ChunkIndex {
    chunk_ids: Vec<String>,
    /// Index into `items` of each chunk's work.
    chunk_items: Vec<u32>,
    items: Vec<String>,
    postings: HashMap<String, Vec<u32>>,
    /// Words of 4+ letters without digits with their chunk counts (what
    /// [`crate::nlp::fuzzy::pick_variants`] reads).
    vocabulary: HashMap<String, i64>,
}

fn build_chunk_index(conn: &Connection, generation_id: &str) -> BibliographyResult<ChunkIndex> {
    let mut stmt = conn
        .prepare(ACTIVE_CHUNK_TEXT_SQL)
        .map_err(|error| err("Failed to prepare chunk index", error))?;
    let mut rows = stmt
        .query([generation_id])
        .map_err(|error| err("Failed to read chunk index", error))?;
    let mut index = ChunkIndex {
        chunk_ids: Vec::new(),
        chunk_items: Vec::new(),
        items: Vec::new(),
        postings: HashMap::new(),
        vocabulary: HashMap::new(),
    };
    let mut item_slots: HashMap<String, u32> = HashMap::new();
    let mut folded = String::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| err("Failed to read chunk index", error))?
    {
        let chunk: u32 = u32::try_from(index.chunk_ids.len())
            .map_err(|error| err("Too many chunks to index", error))?;
        let item_id = row
            .get_ref(1)
            .and_then(|value| value.as_str().map_err(Into::into))
            .map_err(|error| err("Failed to read passage work", error))?;
        let slot = match item_slots.get(item_id) {
            Some(slot) => *slot,
            None => {
                let slot = u32::try_from(index.items.len())
                    .map_err(|error| err("Too many works to index", error))?;
                item_slots.insert(item_id.to_string(), slot);
                index.items.push(item_id.to_string());
                slot
            }
        };
        let text = row
            .get_ref(2)
            .and_then(|value| value.as_str().map_err(Into::into))
            .map_err(|error| err("Failed to read passage text", error))?;
        fold_for_match(text, &mut folded);
        let mut words: Vec<&str> = folded
            .split_ascii_whitespace()
            .filter(|word| word.chars().count() >= 2 && !MATCH_STOPWORDS.contains(word))
            .collect();
        words.sort_unstable();
        words.dedup();
        for word in words {
            match index.postings.get_mut(word) {
                Some(posting) => posting.push(chunk),
                None => {
                    index.postings.insert(word.to_string(), vec![chunk]);
                }
            }
        }
        let id: String = row
            .get(0)
            .map_err(|error| err("Failed to read passage id", error))?;
        index.chunk_ids.push(id);
        index.chunk_items.push(slot);
    }
    index.vocabulary = index
        .postings
        .iter()
        .filter(|(word, _)| word.chars().count() >= 4 && !word.chars().any(|c| c.is_ascii_digit()))
        .map(|(word, posting)| (word.clone(), posting.len() as i64))
        .collect();
    Ok(index)
}

/// The chunk index of the archive and generation, built on first use. A
/// cheap fingerprint (chunk count and highest row id) decides when it is
/// stale; an in-memory archive has no identity to key on and is never cached.
fn chunk_index(
    conn: &Connection,
    generation_id: &str,
) -> BibliographyResult<std::sync::Arc<ChunkIndex>> {
    use std::sync::{Arc, Mutex};
    static CACHE: Mutex<Option<(String, Arc<ChunkIndex>)>> = Mutex::new(None);

    let key = match conn.path() {
        Some(path) if !path.is_empty() => {
            let (count, last): (i64, i64) = conn
                .query_row(
                    "SELECT COUNT(*), COALESCE(MAX(rowid), 0) FROM bibliographic_chunks",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|error| err("Failed to fingerprint chunks", error))?;
            Some(format!("{path}|{generation_id}|{count}|{last}"))
        }
        _ => None,
    };
    if let Some(key) = &key {
        if let Ok(cache) = CACHE.lock() {
            if let Some((cached_key, index)) = cache.as_ref() {
                if cached_key == key {
                    return Ok(Arc::clone(index));
                }
            }
        }
    }
    let index = Arc::new(build_chunk_index(conn, generation_id)?);
    if let Some(key) = key {
        if let Ok(mut cache) = CACHE.lock() {
            *cache = Some((key, Arc::clone(&index)));
        }
    }
    Ok(index)
}

/// Lexical leg: every chunk of the active generation whose text carries the
/// query, exact matches first and close variants after them, each group best
/// first. The index nominates the chunks holding the words (or a variant of
/// them); each is judged on its real text.
fn lexical_passage_candidates(
    conn: &Connection,
    index: &ChunkIndex,
    query: &LexicalQuery,
    allowed: Option<&HashSet<String>>,
    depth: usize,
) -> BibliographyResult<Vec<LexicalCandidate>> {
    // Chunks holding enough of the words (each a word prefix) to match
    // exactly, plus those holding any variant.
    let mut held: HashMap<u32, usize> = HashMap::new();
    for word in &query.words {
        let mut chunks: Vec<u32> = index
            .postings
            .iter()
            .filter(|(candidate, _)| candidate.starts_with(word.as_str()))
            .flat_map(|(_, posting)| posting.iter().copied())
            .collect();
        chunks.sort_unstable();
        chunks.dedup();
        for chunk in chunks {
            *held.entry(chunk).or_default() += 1;
        }
    }
    let mut nominated: HashSet<u32> = held
        .into_iter()
        .filter(|(_, count)| query.required > 0 && *count >= query.required)
        .map(|(chunk, _)| chunk)
        .collect();
    for variant in &query.variants {
        if let Some(posting) = index.postings.get(variant) {
            nominated.extend(posting.iter().copied());
        }
    }

    let tombstoned: HashSet<String> = {
        let mut stmt = conn
            .prepare("SELECT item_id FROM zotero_item_tombstones")
            .map_err(|error| err("Failed to prepare tombstones", error))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| err("Failed to read tombstones", error))?
            .collect::<Result<_, _>>()
            .map_err(|error| err("Failed to collect tombstones", error))?;
        rows
    };
    let mut text_stmt = conn
        .prepare("SELECT text_content FROM bibliographic_chunks WHERE id = ?1")
        .map_err(|error| err("Failed to prepare passage text", error))?;
    let mut matched: Vec<(MatchRank, LexicalCandidate)> = Vec::new();
    let mut folded = String::new();
    for chunk in nominated {
        let item_id = &index.items[index.chunk_items[chunk as usize] as usize];
        if tombstoned.contains(item_id) || allowed.is_some_and(|set| !set.contains(item_id)) {
            continue;
        }
        let chunk_id = &index.chunk_ids[chunk as usize];
        let text: Option<String> = text_stmt
            .query_row([chunk_id], |row| row.get(0))
            .optional()
            .map_err(|error| err("Failed to read passage text", error))?;
        let Some(text) = text else {
            continue;
        };
        fold_for_match(&text, &mut folded);
        let Some((kind, terms, rank)) = query.score(&folded) else {
            continue;
        };
        matched.push((
            rank,
            LexicalCandidate {
                chunk_id: chunk_id.clone(),
                kind,
                terms,
            },
        ));
    }
    matched.sort_by(|a, b| {
        let exact = |kind: PassageMatchKind| kind == PassageMatchKind::Exact;
        exact(b.1.kind)
            .cmp(&exact(a.1.kind))
            .then_with(|| b.0 .0.cmp(&a.0 .0))
            .then_with(|| b.0 .1.cmp(&a.0 .1))
            .then_with(|| {
                b.0 .2
                    .partial_cmp(&a.0 .2)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.1.chunk_id.cmp(&b.1.chunk_id))
    });
    matched.truncate(depth);
    Ok(matched
        .into_iter()
        .map(|(_, candidate)| candidate)
        .collect())
}

/// Cosine similarity of `query` (pre-scaled to unit length) against a stored
/// little-endian f32 blob, read in place. Eight independent lanes let the
/// compiler vectorize what a single running sum cannot; the result is a
/// ranking key, so single precision is plenty (the shown score is recomputed
/// exactly for the few passages that reach the answer).
fn unit_similarity_to_blob(unit_query: &[f32], blob: &[u8]) -> Option<f32> {
    if unit_query.is_empty() || blob.len() != unit_query.len() * 4 {
        return None;
    }
    let mut dot = [0.0_f32; 8];
    let mut mag = [0.0_f32; 8];
    let mut query_lanes = unit_query.chunks_exact(8);
    let mut blob_lanes = blob.chunks_exact(32);
    for (q, b) in (&mut query_lanes).zip(&mut blob_lanes) {
        for lane in 0..8 {
            let value = f32::from_le_bytes([
                b[lane * 4],
                b[lane * 4 + 1],
                b[lane * 4 + 2],
                b[lane * 4 + 3],
            ]);
            dot[lane] += q[lane] * value;
            mag[lane] += value * value;
        }
    }
    let mut dot_sum: f32 = dot.iter().sum();
    let mut mag_sum: f32 = mag.iter().sum();
    for (q, b) in query_lanes
        .remainder()
        .iter()
        .zip(blob_lanes.remainder().chunks_exact(4))
    {
        let value = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        dot_sum += q * value;
        mag_sum += value * value;
    }
    if mag_sum <= 0.0 || !mag_sum.is_finite() || !dot_sum.is_finite() {
        return None;
    }
    Some(dot_sum / mag_sum.sqrt())
}

/// Vector leg: cosine similarity against every fresh chunk vector of the
/// active generation, read in place from the row and never decoded or kept.
/// Memory is bounded by the leg depth, not by the corpus.
fn vector_passage_candidates(
    conn: &Connection,
    generation_id: &str,
    query_vector: &[f32],
    allowed: Option<&HashSet<String>>,
    depth: usize,
) -> BibliographyResult<Vec<(String, f64)>> {
    let norm = query_vector
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt();
    if norm == 0.0 || !norm.is_finite() {
        return Ok(Vec::new());
    }
    let unit_query: Vec<f32> = query_vector
        .iter()
        .map(|value| (f64::from(*value) / norm) as f32)
        .collect();
    let mut stmt = conn
        .prepare(
            "SELECT e.chunk_id, c.item_id, e.embedding
             FROM bibliographic_chunk_embeddings e
             JOIN bibliographic_chunks c ON c.id = e.chunk_id
             LEFT JOIN zotero_item_tombstones t ON t.item_id = c.item_id
             WHERE e.generation_id = ?1 AND t.item_id IS NULL
               AND e.input_hash = c.text_hash",
        )
        .map_err(|error| err("Failed to prepare vector passages", error))?;
    let mut rows = stmt
        .query([generation_id])
        .map_err(|error| err("Failed to read vector passages", error))?;
    let order = |a: &(String, f64), b: &(String, f64)| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    };
    let mut best: Vec<(String, f64)> = Vec::new();
    // Below the current cut-off nothing can enter the leg: skip the id
    // allocation for the overwhelming majority of rows.
    let mut floor = f64::NEG_INFINITY;
    while let Some(row) = rows
        .next()
        .map_err(|error| err("Failed to read vector passages", error))?
    {
        let item_id = row
            .get_ref(1)
            .and_then(|value| value.as_str().map_err(Into::into))
            .map_err(|error| err("Failed to read passage work", error))?;
        if allowed.is_some_and(|set| !set.contains(item_id)) {
            continue;
        }
        let blob = row
            .get_ref(2)
            .and_then(|value| value.as_blob().map_err(Into::into))
            .map_err(|error| err("Failed to read passage vector", error))?;
        let Some(similarity) = unit_similarity_to_blob(&unit_query, blob) else {
            continue;
        };
        let similarity = f64::from(similarity);
        if similarity < floor {
            continue;
        }
        let chunk_id: String = row
            .get(0)
            .map_err(|error| err("Failed to read passage id", error))?;
        best.push((chunk_id, similarity));
        if best.len() >= depth * 4 {
            best.sort_by(order);
            best.truncate(depth);
            floor = best.last().map_or(f64::NEG_INFINITY, |last| last.1);
        }
    }
    best.sort_by(order);
    best.truncate(depth);
    Ok(best)
}

/// Passage search over EVERY chunk of the active generation, never through a
/// pre-selected set of works: a lexical leg over the chunk text (an exact
/// phrase ranks first; with `fuzzy`, close variants of the words follow the
/// exact matches) and a vector leg over the chunk vectors, fused by rank
/// only. Stale chunk vectors (text moved after embed) and tombstoned works
/// never surface. Without an active generation nothing is indexed: the
/// answer is empty and the embedder never runs. When the query cannot be
/// embedded the lexical leg still answers; only if it finds nothing either
/// is the failure reported (`search_unavailable`).
#[allow(clippy::too_many_arguments)]
pub fn search_passages(
    conn: &Connection,
    contract_hash: &str,
    query_text: &str,
    top_chunks_per_work: usize,
    top_k: usize,
    filters: &WorkFilters,
    fuzzy: bool,
    embed_query: &dyn Fn(&str) -> Result<Vec<f32>, String>,
) -> BibliographyResult<Vec<PassageHit>> {
    if query_text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let active = crate::bibliography::generation::active_generation(conn, contract_hash)?;
    let Some(active) = active else {
        return Ok(Vec::new());
    };
    let (query_vector, embed_error) = match embed_query(query_text) {
        Ok(vector) if vector.iter().all(|value| value.is_finite()) => (Some(vector), None),
        Ok(_) => (
            None,
            Some("the query embedding has non-finite values".to_string()),
        ),
        Err(error) => (None, Some(error)),
    };
    let allowed = eligible_items(conn, filters)?;
    let lexical = match LexicalQuery::new(query_text) {
        Some(mut query) => {
            let index = chunk_index(conn, &active.id)?;
            if fuzzy {
                query = query.with_variants(&index.vocabulary);
            }
            lexical_passage_candidates(conn, &index, &query, allowed.as_ref(), PASSAGE_LEG_DEPTH)?
        }
        None => Vec::new(),
    };
    let vector = match &query_vector {
        Some(query_vector) => vector_passage_candidates(
            conn,
            &active.id,
            query_vector,
            allowed.as_ref(),
            PASSAGE_LEG_DEPTH,
        )?,
        None => Vec::new(),
    };
    if let Some(error) = embed_error {
        if lexical.is_empty() {
            return Err(crate::bibliography::repository::BibliographyError::new(
                "search_unavailable",
                format!("Failed to embed passage query: {error}"),
            ));
        }
    }

    // Rank fusion: only positions count, never the legs' own scores. Ties go
    // to the chunk the lexical leg ranked higher (an exact phrase before an
    // equally ranked vector neighbour), then to the id.
    let mut fused: HashMap<&str, (f64, usize)> = HashMap::new();
    let mut why: HashMap<&str, &LexicalCandidate> = HashMap::new();
    for (rank, candidate) in lexical.iter().enumerate() {
        let entry = fused
            .entry(candidate.chunk_id.as_str())
            .or_insert((0.0, usize::MAX));
        entry.0 += 1.0 / (RRF_K + rank as f64);
        entry.1 = rank;
        why.insert(candidate.chunk_id.as_str(), candidate);
    }
    for (rank, (chunk_id, _)) in vector.iter().enumerate() {
        let entry = fused.entry(chunk_id.as_str()).or_insert((0.0, usize::MAX));
        entry.0 += 1.0 / (RRF_K + rank as f64);
    }
    let mut ranked: Vec<(&str, f64, usize)> = fused
        .into_iter()
        .map(|(chunk_id, (score, lexical_rank))| (chunk_id, score, lexical_rank))
        .collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.0.cmp(b.0))
    });

    // Per-work cap first, then the global cut: no single work crowds out
    // every other one.
    let cap = top_chunks_per_work.max(1);
    let mut per_work: HashMap<String, usize> = HashMap::new();
    let mut hits = Vec::new();
    for (chunk_id, fused_score, _) in ranked {
        if hits.len() >= top_k.max(1) {
            break;
        }
        let row: Option<(String, String, i64, String)> = conn
            .query_row(
                "SELECT item_id, text_content, ordinal, attachment_id
                 FROM bibliographic_chunks WHERE id = ?1",
                [chunk_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|error| err("Failed to read passage chunk", error))?;
        let Some((item_id, text, ordinal, attachment_id)) = row else {
            continue;
        };
        let used = per_work.entry(item_id.clone()).or_insert(0);
        if *used >= cap {
            continue;
        }
        let Some(meta) = read_work_meta(conn, &item_id)? else {
            continue;
        };
        *used += 1;
        let vector_score = match &query_vector {
            Some(query_vector) => conn
                .query_row(
                    "SELECT embedding FROM bibliographic_chunk_embeddings
                     WHERE chunk_id = ?1 AND generation_id = ?2",
                    rusqlite::params![chunk_id, active.id],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .optional()
                .map_err(|error| err("Failed to read passage vector", error))?
                .and_then(|blob| crate::nlp::vector::cosine_distance_to_blob(query_vector, &blob))
                .map_or(0.0, |distance| 1.0 - distance),
            None => 0.0,
        };
        let mut spans_stmt = conn
            .prepare(
                "SELECT page_number, start_char, end_char FROM bibliographic_chunk_spans
                 WHERE chunk_id = ?1 ORDER BY page_number, start_char",
            )
            .map_err(|error| err("Failed to prepare passage spans", error))?;
        let spans = spans_stmt
            .query_map([chunk_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|error| err("Failed to read passage spans", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| err("Failed to collect passage spans", error))?;
        drop(spans_stmt);
        let (match_kind, match_terms) = match why.get(chunk_id) {
            Some(candidate) => (candidate.kind, candidate.terms.clone()),
            None => (PassageMatchKind::Meaning, Vec::new()),
        };
        hits.push(PassageHit {
            chunk_id: chunk_id.to_string(),
            item_id,
            item_key: meta.item_key,
            library_id: meta.library_id,
            title: meta.title,
            attachment_id,
            ordinal,
            text,
            spans,
            vector_score,
            match_kind,
            match_terms,
            fused_score,
            generation_id: active.id.clone(),
            contract_hash: active.contract_hash.clone(),
        });
    }
    Ok(hits)
}

#[cfg(test)]
pub(crate) mod passage_tests {
    use super::super::generation::{
        begin_index_generation, complete_index_generation, note_generation_progress,
        register_embedding_contract, set_generation_manifest, EmbeddingContractRow,
    };
    use super::{search_passages, PassageHit, PassageMatchKind, WorkFilters};
    use rusqlite::Connection;

    pub(crate) const CONTRACT: &str = "contract-passages";
    const FAKE_MODEL: &str = "fake/model";

    pub(crate) fn passage_db() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0040_bibliography_catalog.sql"
        ))
        .expect("apply catalog foundation");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0041_bibliography_relations.sql"
        ))
        .expect("apply relations");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0047_bibliographic_semantic_profiles.sql"
        ))
        .expect("apply profiles table");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0049_bibliographic_index_generations.sql"
        ))
        .expect("apply generations");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0054_bibliographic_chunks.sql"
        ))
        .expect("apply chunks");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0055_bibliographic_chunk_embeddings.sql"
        ))
        .expect("apply chunk embeddings");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0051_bibliographic_profile_fts.sql"
        ))
        .expect("apply profile FTS");
        // Test-only stub of the post-0050 work-vector shape: the inner
        // work-level search always reads this table, even when the test
        // leaves it empty. The real file stays pinned by the store tests.
        conn.execute_batch(
            "CREATE TABLE bibliographic_item_embeddings (
               item_id TEXT NOT NULL,
               generation_id TEXT NOT NULL,
               embedding_contract TEXT NOT NULL,
               embedding_model TEXT NOT NULL,
               dimensions INTEGER NOT NULL,
               embedding BLOB NOT NULL,
               input_hash TEXT NOT NULL,
               profile_revision INTEGER NOT NULL,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (item_id, generation_id)
             );",
        )
        .expect("stub work vectors");
        conn
    }

    fn contract_row() -> EmbeddingContractRow {
        EmbeddingContractRow {
            contract_hash: CONTRACT.to_string(),
            provider: "api".to_string(),
            model: FAKE_MODEL.to_string(),
            dimensions: 4,
            chunking_contract: "test-chunking".to_string(),
        }
    }

    pub(crate) fn seed_item(
        conn: &mut Connection,
        connection_id: &str,
        library_suffix: &str,
        item_key: &str,
        title: &str,
    ) -> (String, String) {
        use super::super::repository::{
            upsert_connection, upsert_item, upsert_library, BibliographicItemInput, LibraryType,
            SourceOrigin, UpsertConnection, UpsertLibrary,
        };
        let source = upsert_connection(
            conn,
            UpsertConnection {
                id: format!("conn-{connection_id}"),
                source_origin: SourceOrigin::Local,
                source_instance_id: None,
                endpoint: Some("http://synthetic.invalid".to_string()),
                capabilities_json: r#"{"read":true}"#.to_string(),
            },
        )
        .expect("connection");
        let library = upsert_library(
            conn,
            UpsertLibrary {
                connection_id: source.id,
                library_type: LibraryType::User,
                library_id: format!("lib-{library_suffix}"),
                name: format!("Personal {library_suffix}"),
                last_modified_version: Some(7),
            },
        )
        .expect("library");
        let item = upsert_item(
            conn,
            &library.id,
            BibliographicItemInput {
                item_key: item_key.to_string(),
                item_version: Some(3),
                native_json_snapshot: serde_json::json!({"key": item_key, "version": 3})
                    .to_string(),
                csl_json_snapshot: serde_json::json!({
                    "id": item_key, "type": "book", "title": title,
                    "issued": { "date-parts": [[2020]] },
                })
                .to_string(),
                title: Some(title.to_string()),
                ..Default::default()
            },
        )
        .expect("catalog item");
        (library.id, item.id)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn seed_chunk(
        conn: &Connection,
        item_id: &str,
        ordinal: i64,
        text: &str,
        page: i64,
        generation_id: &str,
        vector: &[f32; 4],
        input_hash: Option<&str>,
    ) -> String {
        let chunk_id = format!("{item_id}:{ordinal:06}");
        let attachment_id = format!("att-{chunk_id}");
        conn.execute(
            "INSERT INTO zotero_attachments
               (id, item_id, attachment_key, native_json_snapshot, created_at, updated_at, verified_at)
             VALUES (?1, ?2, ?3, '{}', 1, 1, 1)",
            rusqlite::params![attachment_id, item_id, format!("key-{ordinal}")],
        )
        .expect("seed attachment");
        let hash = super::super::profile::profile_input_hash(text);
        conn.execute(
            "INSERT INTO bibliographic_chunks
               (id, item_id, attachment_id, ordinal, text_content, text_hash,
                chunking_contract, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'test-chunking', 1, 1)",
            rusqlite::params![chunk_id, item_id, attachment_id, ordinal, text, hash],
        )
        .expect("seed chunk");
        conn.execute(
            "INSERT INTO bibliographic_chunk_spans (chunk_id, page_number, start_char, end_char)
             VALUES (?1, ?2, 0, ?3)",
            rusqlite::params![chunk_id, page, text.chars().count() as i64],
        )
        .expect("seed span");
        let blob: Vec<u8> = vector
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        conn.execute(
            "INSERT INTO bibliographic_chunk_embeddings
               (chunk_id, generation_id, embedding_contract, embedding_model,
                dimensions, embedding, input_hash, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 4, ?5, ?6, 1, 1)",
            rusqlite::params![
                chunk_id,
                generation_id,
                CONTRACT,
                FAKE_MODEL,
                blob,
                input_hash.unwrap_or(&hash)
            ],
        )
        .expect("seed chunk vector");
        chunk_id
    }

    pub(crate) fn activate_gen(conn: &mut Connection, generation_id: &str) -> String {
        register_embedding_contract(conn, &contract_row(), 1).expect("register");
        let staging = begin_index_generation(conn, CONTRACT, generation_id, 10).expect("begin");
        // Attach semantics may return a previously opened staging row.
        let staging_id: String = conn
            .query_row(
                "SELECT id FROM bibliographic_index_generations
                 WHERE contract_hash = ?1 AND status = 'staging'",
                [CONTRACT],
                |row| row.get(0),
            )
            .expect("staging id");
        let _ = staging;
        set_generation_manifest(conn, &staging_id, 1, 11).expect("manifest");
        note_generation_progress(conn, &staging_id).expect("progress");
        complete_index_generation(conn, &staging_id, 20).expect("activate");
        staging_id
    }

    fn search_all(
        conn: &Connection,
        text: &str,
        embed: &dyn Fn(&str) -> Result<Vec<f32>, String>,
    ) -> Vec<PassageHit> {
        search_passages(
            conn,
            CONTRACT,
            text,
            3,
            10,
            &WorkFilters::default(),
            true,
            embed,
        )
        .expect("passage search")
    }

    #[test]
    fn passages_rank_chunks_within_candidate_works() {
        let mut conn = passage_db();
        let (_lib_a, item_a) = seed_item(&mut conn, "pa", "a", "KA0001", "Obra A");
        let (_lib_b, item_b) = seed_item(&mut conn, "pb", "b", "KB0001", "Obra B");
        // Profiles make both works work-level candidates through the
        // lexical leg ("compartido" appears in both profile texts).
        for (item_id, title) in [(&item_a, "Obra A"), (&item_b, "Obra B")] {
            let text = format!("{title} con vocabulario compartido distintivo.");
            let hash = super::super::profile::profile_input_hash(&text);
            conn.execute(
                "INSERT INTO bibliographic_semantic_profiles
                   (item_id, profile_revision, template_version, canonical_text,
                    input_hash, field_provenance_json, created_at, updated_at)
                 VALUES (?1, 1, 'bibliography-profile-v1', ?2, ?3, '[]', 1, 1)",
                rusqlite::params![item_id, text, hash],
            )
            .expect("seed profile");
        }
        let gen = activate_gen(&mut conn, "gen-passages");
        // Chunk vectors: B's chunk is parallel to the query, A's is not.
        let chunk_a = seed_chunk(
            &conn,
            &item_a,
            0,
            "Texto de la obra A sin similitud.",
            1,
            &gen,
            &[1.0, 0.0, 0.0, 0.0],
            None,
        );
        let chunk_b = seed_chunk(
            &conn,
            &item_b,
            0,
            "Texto de la obra B relevante.",
            1,
            &gen,
            &[0.0, 1.0, 0.0, 0.0],
            None,
        );

        let hits = search_all(&conn, "compartido", &|_| Ok(vec![0.0, 1.0, 0.0, 0.0]));
        assert_eq!(hits.len(), 2, "both works surface passages, got {hits:?}");
        assert_eq!(hits[0].chunk_id, chunk_b, "the parallel chunk ranks first");
        assert_eq!(hits[0].item_id, item_b);
        assert!((hits[0].vector_score - 1.0).abs() < 1e-9);
        assert_eq!(hits[0].generation_id, gen);
        assert_eq!(hits[0].contract_hash, CONTRACT);
        assert_eq!(hits[0].spans, vec![(1, 0, 29)]);
        assert!(
            hits[0].fused_score > 0.0,
            "the fusion score travels with the hit"
        );
        assert_eq!(hits[1].chunk_id, chunk_a);
    }

    #[test]
    fn passages_exclude_stale_chunk_vectors() {
        let mut conn = passage_db();
        let (_lib, item) = seed_item(&mut conn, "ps", "s", "KS0001", "Obra sola");
        let text = "Texto original del fragmento con longitud.";
        let _hash = super::super::profile::profile_input_hash(text);
        conn.execute(
            "INSERT INTO bibliographic_semantic_profiles
               (item_id, profile_revision, template_version, canonical_text,
                input_hash, field_provenance_json, created_at, updated_at)
             VALUES (?1, 1, 'bibliography-profile-v1', 'Título: Obra sola', 'hprof', '[]', 1, 1)",
            [&item],
        )
        .expect("seed profile");
        let gen = activate_gen(&mut conn, "gen-stale");
        // The vector stamps an older text; the chunk row moved on.
        seed_chunk(
            &conn,
            &item,
            0,
            text,
            1,
            &gen,
            &[0.0, 1.0, 0.0, 0.0],
            Some("hash-older"),
        );
        conn.execute(
            "UPDATE bibliographic_chunks SET text_content = 'Texto corregido del fragmento.', text_hash = 'hash-newer' WHERE item_id = ?1",
            [&item],
        )
        .expect("move chunk text");

        let hits = search_all(&conn, "compartido", &|_| Ok(vec![0.0, 1.0, 0.0, 0.0]));
        assert!(
            hits.is_empty(),
            "a vector computed from older text must not surface, got {hits:?}"
        );
    }

    #[test]
    fn passages_cap_chunks_per_work() {
        let mut conn = passage_db();
        let (_lib_a, item_a) = seed_item(&mut conn, "pc", "c", "KC0001", "Obra C");
        let (_lib_b, item_b) = seed_item(&mut conn, "pd", "d", "KD0001", "Obra D");
        for (item_id, title) in [(&item_a, "Obra C"), (&item_b, "Obra D")] {
            let text = format!("{title} vocabulario compartido.");
            let hash = super::super::profile::profile_input_hash(&text);
            conn.execute(
                "INSERT INTO bibliographic_semantic_profiles
                   (item_id, profile_revision, template_version, canonical_text,
                    input_hash, field_provenance_json, created_at, updated_at)
                 VALUES (?1, 1, 'bibliography-profile-v1', ?2, ?3, '[]', 1, 1)",
                rusqlite::params![item_id, text, hash],
            )
            .expect("seed profile");
        }
        let gen = activate_gen(&mut conn, "gen-cap");
        // A dominates on similarity with three chunks; D has one mid chunk.
        seed_chunk(
            &conn,
            &item_a,
            0,
            "Fragmento C cero paralelo.",
            1,
            &gen,
            &[1.0, 0.0, 0.0, 0.0],
            None,
        );
        seed_chunk(
            &conn,
            &item_a,
            1,
            "Fragmento C uno paralelo.",
            2,
            &gen,
            &[1.0, 0.0, 0.0, 0.0],
            None,
        );
        seed_chunk(
            &conn,
            &item_a,
            2,
            "Fragmento C dos paralelo.",
            3,
            &gen,
            &[1.0, 0.0, 0.0, 0.0],
            None,
        );
        seed_chunk(
            &conn,
            &item_b,
            0,
            "Fragmento D medio.",
            1,
            &gen,
            &[0.7, 0.7, 0.0, 0.0],
            None,
        );

        let hits = search_passages(
            &conn,
            CONTRACT,
            "compartido",
            1,
            2,
            &WorkFilters::default(),
            true,
            &|_| Ok(vec![1.0, 0.0, 0.0, 0.0]),
        )
        .expect("passage search");
        assert_eq!(hits.len(), 2, "cap one per work, top two overall");
        assert_eq!(hits[0].item_id, item_a);
        assert_eq!(
            hits[1].item_id, item_b,
            "D survives the cap instead of A taking all three"
        );
    }

    /// The owner's case: the work's profile (title/abstract) says nothing
    /// about the phrase, and the vectors of its chunks point elsewhere, but
    /// one chunk contains the phrase verbatim.
    fn seed_phrase_corpus(conn: &mut Connection) -> (String, String, String, String) {
        let (_lib_a, item_a) = seed_item(conn, "pf", "f", "KF0001", "La producción del espacio");
        let (_lib_b, item_b) = seed_item(conn, "pg", "g", "KG0001", "Otra obra distinta");
        let gen = activate_gen(conn, "gen-phrase");
        let phrase_chunk = seed_chunk(
            conn,
            &item_a,
            0,
            "Dos de dichos programas (Plan  Federal y Programa
Dignidad) desataron la guerra por las casas.",
            2,
            &gen,
            &[0.0, 0.0, 1.0, 0.0],
            None,
        );
        seed_chunk(
            conn,
            &item_a,
            1,
            "Texto sin relación con la consulta.",
            3,
            &gen,
            &[0.0, 0.0, 0.0, 1.0],
            None,
        );
        let mut best_vector_chunk = String::new();
        for ordinal in 0..4 {
            let id = seed_chunk(
                conn,
                &item_b,
                ordinal,
                &format!("Fragmento {ordinal} de otra obra sobre cosas varias."),
                1 + ordinal,
                &gen,
                &[1.0, 0.0, 0.0, 0.0],
                None,
            );
            if ordinal == 0 {
                best_vector_chunk = id;
            }
        }
        (item_a, item_b, phrase_chunk, best_vector_chunk)
    }

    #[test]
    fn passages_find_a_phrase_in_a_work_whose_profile_does_not_match() {
        let mut conn = passage_db();
        let (item_a, _item_b, phrase_chunk, _best) = seed_phrase_corpus(&mut conn);
        let hits = search_all(&conn, "Plan Federal y Programa Dignidad", &|_| {
            Ok(vec![1.0, 0.0, 0.0, 0.0])
        });
        let found = hits
            .iter()
            .find(|hit| hit.chunk_id == phrase_chunk)
            .unwrap_or_else(|| panic!("the phrase chunk must be returned, got {hits:?}"));
        assert_eq!(found.item_id, item_a);
        assert_eq!(found.match_kind, PassageMatchKind::Exact);
        assert_eq!(
            found.match_terms,
            vec!["plan", "federal", "programa", "dignidad"]
        );
        assert_eq!(found.spans.first().map(|span| span.0), Some(2));
        assert_eq!(
            hits[0].chunk_id, phrase_chunk,
            "an exact phrase ranks first, ahead of an equally ranked vector hit"
        );
    }

    #[test]
    fn passages_find_a_phrase_without_the_embedder() {
        let mut conn = passage_db();
        let (_a, _b, phrase_chunk, _best) = seed_phrase_corpus(&mut conn);
        let hits = search_all(&conn, "plan federal y programa dignidad", &|_| {
            Err("provider down".to_string())
        });
        assert_eq!(hits.len(), 1, "lexical-only answer, got {hits:?}");
        assert_eq!(hits[0].chunk_id, phrase_chunk);
    }

    #[test]
    fn passages_lexical_leg_ignores_accents_and_case() {
        let mut conn = passage_db();
        let (_a, _b, phrase_chunk, _best) = seed_phrase_corpus(&mut conn);
        let hits = search_all(&conn, "GUERRA por las CÁSAS", &|_| {
            Ok(vec![1.0, 0.0, 0.0, 0.0])
        });
        assert!(hits.iter().any(|hit| hit.chunk_id == phrase_chunk));
    }

    #[test]
    fn passages_respect_library_filters_in_both_legs() {
        let mut conn = passage_db();
        let (item_a, item_b, phrase_chunk, best) = seed_phrase_corpus(&mut conn);
        let lib_b: String = conn
            .query_row(
                "SELECT library_id FROM bibliographic_items WHERE id = ?1",
                [&item_b],
                |row| row.get(0),
            )
            .expect("library of b");
        let filters = WorkFilters {
            library_ids: vec![lib_b],
            ..WorkFilters::default()
        };
        let hits = search_passages(
            &conn,
            CONTRACT,
            "Plan Federal y Programa Dignidad",
            3,
            10,
            &filters,
            true,
            &|_| Ok(vec![1.0, 0.0, 0.0, 0.0]),
        )
        .expect("passage search");
        assert!(hits.iter().all(|hit| hit.item_id == item_b), "{hits:?}");
        assert!(hits.iter().any(|hit| hit.chunk_id == best));
        assert!(hits.iter().all(|hit| hit.chunk_id != phrase_chunk));
        let _ = item_a;
    }

    /// Two chunks spell a name "Crocitto" (an OCR doubling); the reader
    /// types "Croitto". Another chunk spells it as typed.
    fn seed_misreading_corpus(conn: &mut Connection) -> (String, String, String) {
        let (_lib, item) = seed_item(conn, "ph", "h", "KH0001", "Estudios del chamanismo");
        let gen = activate_gen(conn, "gen-fuzzy");
        let misread_a = seed_chunk(
            conn,
            &item,
            0,
            "El trabajo de Crocitto sobre los mapuche es citado a menudo.",
            1,
            &gen,
            &[0.0, 0.0, 1.0, 0.0],
            None,
        );
        seed_chunk(
            conn,
            &item,
            1,
            "Según Crocitto, la ceremonia cambia con el tiempo.",
            2,
            &gen,
            &[0.0, 0.0, 0.0, 1.0],
            None,
        );
        let exact = seed_chunk(
            conn,
            &item,
            2,
            "Croitto escribió sobre el mismo tema.",
            3,
            &gen,
            &[0.0, 1.0, 0.0, 0.0],
            None,
        );
        (item, misread_a, exact)
    }

    fn text_matches(hits: &[PassageHit]) -> Vec<(&str, PassageMatchKind)> {
        hits.iter()
            .filter(|hit| hit.match_kind != PassageMatchKind::Meaning)
            .map(|hit| (hit.chunk_id.as_str(), hit.match_kind))
            .collect()
    }

    #[test]
    fn approximate_matching_adds_close_variants_after_the_exact_matches() {
        let mut conn = passage_db();
        let (_item, misread, exact) = seed_misreading_corpus(&mut conn);
        let hits = search_passages(
            &conn,
            CONTRACT,
            "Croitto",
            5,
            10,
            &WorkFilters::default(),
            true,
            &|_| Ok(vec![1.0, 0.0, 0.0, 0.0]),
        )
        .expect("passage search");
        let found = text_matches(&hits);
        assert_eq!(found[0], (exact.as_str(), PassageMatchKind::Exact));
        assert!(
            found.contains(&(misread.as_str(), PassageMatchKind::Approximate)),
            "the misreading is found as a variant, got {found:?}"
        );
        let approximate = hits
            .iter()
            .find(|hit| hit.chunk_id == misread)
            .expect("misreading hit");
        assert_eq!(approximate.match_terms, vec!["crocitto"]);
        let exact_hit = hits
            .iter()
            .find(|hit| hit.chunk_id == exact)
            .expect("exact");
        assert_eq!(exact_hit.match_terms, vec!["croitto"]);
    }

    #[test]
    fn approximate_matching_is_off_when_not_asked_for() {
        let mut conn = passage_db();
        let (_item, _misread, exact) = seed_misreading_corpus(&mut conn);
        let hits = search_passages(
            &conn,
            CONTRACT,
            "Croitto",
            5,
            10,
            &WorkFilters::default(),
            false,
            &|_| Ok(vec![1.0, 0.0, 0.0, 0.0]),
        )
        .expect("passage search");
        assert_eq!(
            text_matches(&hits),
            vec![(exact.as_str(), PassageMatchKind::Exact)]
        );
    }

    #[test]
    fn a_passage_found_only_by_its_vector_is_labelled_meaning() {
        let mut conn = passage_db();
        let (_a, _b, _phrase, best) = seed_phrase_corpus(&mut conn);
        let hits = search_all(&conn, "Plan Federal y Programa Dignidad", &|_| {
            Ok(vec![1.0, 0.0, 0.0, 0.0])
        });
        let neighbour = hits
            .iter()
            .find(|hit| hit.chunk_id == best)
            .expect("neighbour");
        assert_eq!(neighbour.match_kind, PassageMatchKind::Meaning);
        assert!(neighbour.match_terms.is_empty());
    }

    #[test]
    fn passages_stay_empty_without_a_queryable_space() {
        let mut conn = passage_db();
        let (_lib, item) = seed_item(&mut conn, "pe", "e", "KE0001", "Obra E");
        let text = "Texto con vocabulario compartido.";
        let hash = super::super::profile::profile_input_hash(text);
        conn.execute(
            "INSERT INTO bibliographic_semantic_profiles
               (item_id, profile_revision, template_version, canonical_text,
                input_hash, field_provenance_json, created_at, updated_at)
             VALUES (?1, 1, 'bibliography-profile-v1', ?2, ?3, '[]', 1, 1)",
            rusqlite::params![item, text, hash],
        )
        .expect("seed profile");

        let hits = search_passages(
            &conn,
            CONTRACT,
            "compartido",
            3,
            10,
            &WorkFilters::default(),
            true,
            &|_| panic!("the embedder must never run without a queryable space"),
        )
        .expect("passage search");
        assert!(
            hits.is_empty(),
            "no active generation means nothing is indexed: no hits"
        );
    }
}

// ── E4d-WU2: explicit passage expansion (RED stubs) ────────────────────────

/// One neighboring chunk in reading order.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkNeighbor {
    pub chunk_id: String,
    pub ordinal: i64,
    pub text: String,
}

/// One page of context with the chunk's character ranges marked.
#[derive(Debug, Clone, PartialEq)]
pub struct PageContext {
    pub page_number: i64,
    pub text: String,
    pub highlights: Vec<(i64, i64)>,
}

/// A chunk with its reading neighbors, page context, and vector
/// provenance: everything the highlight surface needs without a second
/// round trip.
#[derive(Debug, Clone, PartialEq)]
pub struct PassageExpansion {
    pub chunk_id: String,
    pub item_id: String,
    pub attachment_id: String,
    pub item_key: String,
    pub library_id: String,
    pub title: String,
    pub ordinal: i64,
    pub text: String,
    pub spans: Vec<(i64, i64, i64)>,
    pub chunking_contract: String,
    pub previous: Option<ChunkNeighbor>,
    pub next: Option<ChunkNeighbor>,
    pub pages: Vec<PageContext>,
    /// Every (generation, contract) stamped with a vector for this chunk.
    pub vectors: Vec<(String, String)>,
}

/// Expands one chunk: neighbors by ordinal within the work, page texts
/// with highlight ranges, vector provenance. Unknown chunks answer None.
/// Missing page rows are skipped, never fatal: context degrades to the
/// chunk text instead of failing the highlight.
pub fn expand_passage(
    conn: &Connection,
    chunk_id: &str,
) -> BibliographyResult<Option<PassageExpansion>> {
    let row: Option<(String, String, i64, String, String, String)> = conn
        .query_row(
            "SELECT c.item_id, c.attachment_id, c.ordinal, c.text_content, c.text_hash,
                    c.chunking_contract
             FROM bibliographic_chunks c WHERE c.id = ?1",
            [chunk_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(|error| err("Failed to read expanded chunk", error))?;
    let Some((item_id, attachment_id, ordinal, text, _hash, chunking_contract)) = row else {
        return Ok(None);
    };
    let Some(meta) = read_work_meta(conn, &item_id)? else {
        return Ok(None);
    };
    let spans: Vec<(i64, i64, i64)> = conn
        .prepare(
            "SELECT page_number, start_char, end_char FROM bibliographic_chunk_spans
             WHERE chunk_id = ?1 ORDER BY page_number, start_char",
        )
        .map_err(|error| err("Failed to prepare expansion spans", error))?
        .query_map([chunk_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|error| err("Failed to read expansion spans", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| err("Failed to collect expansion spans", error))?;
    // Neighbors by ordinal within the work: at most one each side.
    let neighbor = |direction: &str| -> BibliographyResult<Option<ChunkNeighbor>> {
        let comparison = if direction == "previous" { "<" } else { ">" };
        let ordering = if direction == "previous" {
            "DESC"
        } else {
            "ASC"
        };
        let row: Option<(String, i64, String)> = conn
            .query_row(
                &format!(
                    "SELECT id, ordinal, text_content FROM bibliographic_chunks
                     WHERE item_id = ?1 AND ordinal {comparison} ?2 ORDER BY ordinal {ordering} LIMIT 1"
                ),
                rusqlite::params![item_id, ordinal],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| err("Failed to read chunk neighbor", error))?;
        Ok(row.map(|(chunk_id, ordinal, text)| ChunkNeighbor {
            chunk_id,
            ordinal,
            text,
        }))
    };
    let previous = neighbor("previous")?;
    let next = neighbor("next")?;
    // Page context: current preferred text per spanned page with this
    // chunk's ranges marked. Missing pages degrade to chunk text.
    let mut pages = Vec::new();
    let mut seen_pages: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for (page_number, _, _) in &spans {
        if !seen_pages.insert(*page_number) {
            continue;
        }
        let page_text: Option<String> = conn
            .query_row(
                "SELECT text_content FROM bibliographic_page_texts
                 WHERE attachment_id IN
                   (SELECT attachment_id FROM bibliographic_chunks WHERE id = ?1)
                   AND page_number = ?2
                 ORDER BY CASE method WHEN 'ocr' THEN 0 ELSE 1 END LIMIT 1",
                rusqlite::params![chunk_id, page_number],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| err("Failed to read expansion page", error))?;
        let Some(page_text) = page_text else {
            continue;
        };
        let highlights: Vec<(i64, i64)> = spans
            .iter()
            .filter(|(page, _, _)| page == page_number)
            .map(|(_, start, end)| (*start, *end))
            .collect();
        pages.push(PageContext {
            page_number: *page_number,
            text: page_text,
            highlights,
        });
    }
    pages.sort_by_key(|page| page.page_number);
    let vectors: Vec<(String, String)> = conn
        .prepare(
            "SELECT generation_id, embedding_contract FROM bibliographic_chunk_embeddings
             WHERE chunk_id = ?1 ORDER BY generation_id",
        )
        .map_err(|error| err("Failed to prepare expansion vectors", error))?
        .query_map([chunk_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| err("Failed to read expansion vectors", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| err("Failed to collect expansion vectors", error))?;
    Ok(Some(PassageExpansion {
        chunk_id: chunk_id.to_string(),
        item_id: item_id.clone(),
        attachment_id: attachment_id.clone(),
        item_key: meta.item_key,
        library_id: meta.library_id,
        title: meta.title,
        ordinal,
        text,
        spans,
        chunking_contract,
        previous,
        next,
        pages,
        vectors,
    }))
}

pub use crate::bibliography::attachment::OriginalKind;

/// The original behind a passage, validated for in-app viewing.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewableOriginal {
    pub kind: OriginalKind,
    /// The canonical file to grant to the viewer: set for a PDF, `None` for
    /// an HTML snapshot, which is shown from the catalog's stored text.
    pub path: Option<std::path::PathBuf>,
}

/// A passage ready to open: the expansion for the highlight surface plus
/// the original validated for in-app viewing (or the reason there is none).
/// Nothing here launches an external program; the command grants the one
/// validated file to the webview and the UI renders the expansion regardless.
#[derive(Debug, Clone, PartialEq)]
pub struct PassageOpenPlan {
    pub expansion: PassageExpansion,
    pub original: Option<ViewableOriginal>,
    /// `(reason, detail)` when no original is viewable: the resolver's or
    /// the validator's stable strings.
    pub reason: Option<(String, String)>,
}

/// Prepares a passage opening without spawning anything: expands the
/// chunk, reads its registered attachment, resolves and validates the file.
/// Unknown chunks fail with `unknown_chunk`; an unviewable original travels
/// as `reason`, never as an error, so the highlight surface still renders.
/// The only path ever produced comes from a registered attachment row.
pub fn prepare_passage_open(
    conn: &Connection,
    chunk_id: &str,
    zotero_data_dir: Option<&str>,
) -> BibliographyResult<PassageOpenPlan> {
    let Some(expansion) = expand_passage(conn, chunk_id)? else {
        return Err(crate::bibliography::repository::BibliographyError::new(
            "unknown_chunk",
            format!("chunk {chunk_id} does not exist"),
        ));
    };
    let attachment =
        crate::bibliography::attachment::attachment_ref_for(conn, &expansion.attachment_id)
            .map_err(|error| {
                crate::bibliography::repository::BibliographyError::new(
                    "sql_error",
                    format!("Failed to read passage attachment: {error}"),
                )
            })?;
    let (original, reason) = match attachment {
        None => (
            None,
            Some((
                "unknown_attachment".to_string(),
                "the passage attachment left the catalog".to_string(),
            )),
        ),
        Some(attachment) if crate::bibliography::attachment::is_html_snapshot(&attachment) => (
            Some(ViewableOriginal {
                kind: OriginalKind::Html,
                path: None,
            }),
            None,
        ),
        Some(attachment) => {
            match crate::bibliography::attachment::resolve_attachment_file(
                &attachment,
                zotero_data_dir,
            ) {
                crate::bibliography::attachment::AttachmentResolution::File(path) => {
                    match crate::bibliography::attachment::validate_pdf_original(
                        &path,
                        attachment.content_type.as_deref(),
                    ) {
                        Ok(canonical) => (
                            Some(ViewableOriginal {
                                kind: OriginalKind::Pdf,
                                path: Some(canonical),
                            }),
                            None,
                        ),
                        Err(reason) => (None, Some(reason)),
                    }
                }
                crate::bibliography::attachment::AttachmentResolution::Unavailable {
                    reason,
                    detail,
                } => (None, Some((reason.to_string(), detail))),
            }
        }
    };
    Ok(PassageOpenPlan {
        expansion,
        original,
        reason,
    })
}

#[cfg(test)]
mod expansion_tests {
    use super::super::retrieval::{expand_passage, PassageExpansion};
    use rusqlite::Connection;

    fn expansion_db() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0040_bibliography_catalog.sql"
        ))
        .expect("apply catalog foundation");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0041_bibliography_relations.sql"
        ))
        .expect("apply relations");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0047_bibliographic_semantic_profiles.sql"
        ))
        .expect("apply profiles table");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0049_bibliographic_index_generations.sql"
        ))
        .expect("apply generations");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0053_bibliographic_page_texts.sql"
        ))
        .expect("apply page texts");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0054_bibliographic_chunks.sql"
        ))
        .expect("apply chunks");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0055_bibliographic_chunk_embeddings.sql"
        ))
        .expect("apply chunk embeddings");
        conn
    }

    fn seed(expansion_seed: &str) -> (Connection, String, String, String) {
        use super::super::repository::{
            upsert_connection, upsert_item, upsert_library, BibliographicItemInput, LibraryType,
            SourceOrigin, UpsertConnection, UpsertLibrary,
        };
        let mut conn = expansion_db();
        let source = upsert_connection(
            &mut conn,
            UpsertConnection {
                id: "conn-1".to_string(),
                source_origin: SourceOrigin::Local,
                source_instance_id: None,
                endpoint: Some("http://synthetic.invalid".to_string()),
                capabilities_json: r#"{"read":true}"#.to_string(),
            },
        )
        .expect("connection");
        let library = upsert_library(
            &mut conn,
            UpsertLibrary {
                connection_id: source.id,
                library_type: LibraryType::User,
                library_id: "0".to_string(),
                name: "Personal".to_string(),
                last_modified_version: Some(7),
            },
        )
        .expect("library");
        let item = upsert_item(
            &mut conn,
            &library.id,
            BibliographicItemInput {
                item_key: "EXP0001".to_string(),
                item_version: Some(3),
                native_json_snapshot: r#"{"key":"EXP0001","version":3}"#.to_string(),
                csl_json_snapshot: serde_json::json!({
                    "id": "EXP0001", "type": "book", "title": "Obra expandible",
                })
                .to_string(),
                title: Some("Obra expandible".to_string()),
                ..Default::default()
            },
        )
        .expect("catalog item");
        conn.execute(
            "INSERT INTO zotero_attachments
               (id, item_id, attachment_key, native_json_snapshot, created_at, updated_at, verified_at)
             VALUES ('att-exp', ?1, 'EXPATT01', '{}', 1, 1, 1)",
            [&item.id],
        )
        .expect("seed attachment");
        // Three chunks across two pages; the middle one spans both.
        for (ordinal, text, hash) in [
            (0i64, "Primer fragmento del documento.", "h0"),
            (1i64, "Segundo fragmento que cruza de pagina.", "h1"),
            (2i64, "Tercer fragmento final.", "h2"),
        ] {
            let chunk_id = format!("{}:{ordinal:06}", item.id);
            conn.execute(
                "INSERT INTO bibliographic_chunks
                   (id, item_id, attachment_id, ordinal, text_content, text_hash,
                    chunking_contract, created_at, updated_at)
                 VALUES (?1, ?2, 'att-exp', ?3, ?4, ?5, 'test-chunking', 1, 1)",
                rusqlite::params![chunk_id, item.id, ordinal, text, hash],
            )
            .expect("seed chunk");
        }
        for (chunk_suffix, page, start, end) in
            [(0, 1, 0, 33), (1, 1, 34, 60), (1, 2, 0, 20), (2, 2, 21, 45)]
        {
            conn.execute(
                "INSERT INTO bibliographic_chunk_spans (chunk_id, page_number, start_char, end_char)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    format!("{}:{chunk_suffix:06}", item.id),
                    page,
                    start,
                    end
                ],
            )
            .expect("seed span");
        }
        for page in [1i64, 2] {
            conn.execute(
                "INSERT INTO bibliographic_page_texts
                   (attachment_id, page_number, method, text_content, text_hash,
                    text_chars, quality, created_at, updated_at)
                 VALUES ('att-exp', ?1, 'native', ?2, ?3, 60, 'rich', 1, 1)",
                rusqlite::params![
                    page,
                    format!("Texto completo de la pagina {page} para contexto."),
                    format!("pagehash-{page}")
                ],
            )
            .expect("seed page text");
        }
        // Only the middle chunk carries a vector.
        conn.execute(
            "INSERT INTO bibliographic_embedding_contracts
               (contract_hash, provider, model, dimensions, chunking_contract, created_at)
             VALUES ('contract-exp', 'api', 'fake/model', 4, 'test-chunking', 1)",
            [],
        )
        .expect("seed contract");
        conn.execute(
            "INSERT INTO bibliographic_index_generations
               (id, contract_hash, status, expected_inputs, completed_inputs, created_at)
             VALUES ('gen-exp', 'contract-exp', 'active', 1, 1, 1)",
            [],
        )
        .expect("seed generation");
        conn.execute(
            "INSERT INTO bibliographic_chunk_embeddings
               (chunk_id, generation_id, embedding_contract, embedding_model,
                dimensions, embedding, input_hash, created_at, updated_at)
             VALUES (?1, 'gen-exp', 'contract-exp', 'fake/model', 4, zeroblob(4), 'h1', 1, 1)",
            [format!("{}:000001", item.id)],
        )
        .expect("seed chunk vector");
        let _ = expansion_seed;
        let middle = format!("{}:000001", item.id);
        (conn, item.id, library.id, middle)
    }

    #[test]
    fn expansion_returns_neighbors_page_context_and_provenance() {
        let (conn, item_id, library_id, middle) = seed("exp-1");
        let expansion: PassageExpansion = expand_passage(&conn, &middle)
            .expect("expand")
            .expect("present");

        assert_eq!(expansion.chunk_id, middle);
        assert_eq!(expansion.item_id, item_id);
        assert_eq!(expansion.item_key, "EXP0001");
        assert_eq!(expansion.library_id, library_id);
        assert_eq!(expansion.title, "Obra expandible");
        assert_eq!(expansion.ordinal, 1);
        assert_eq!(expansion.text, "Segundo fragmento que cruza de pagina.");
        assert_eq!(expansion.spans, vec![(1, 34, 60), (2, 0, 20)]);
        assert_eq!(expansion.chunking_contract, "test-chunking");

        let previous = expansion.previous.expect("previous neighbor");
        assert_eq!(previous.ordinal, 0);
        assert_eq!(previous.text, "Primer fragmento del documento.");
        let next = expansion.next.expect("next neighbor");
        assert_eq!(next.ordinal, 2);
        assert_eq!(next.text, "Tercer fragmento final.");

        assert_eq!(expansion.pages.len(), 2, "both spanned pages surface");
        assert_eq!(expansion.pages[0].page_number, 1);
        assert_eq!(expansion.pages[0].highlights, vec![(34, 60)]);
        assert!(expansion.pages[0].text.contains("pagina 1"));
        assert_eq!(expansion.pages[1].highlights, vec![(0, 20)]);

        assert_eq!(
            expansion.vectors,
            vec![("gen-exp".to_string(), "contract-exp".to_string())]
        );
    }

    #[test]
    fn expansion_edges_have_single_neighbors() {
        let (conn, item_id, _, _) = seed("exp-2");
        let first = expand_passage(&conn, &format!("{item_id}:000000"))
            .expect("expand")
            .expect("present");
        assert!(first.previous.is_none(), "the first chunk has no previous");
        assert!(first.next.is_some());
        assert_eq!(first.vectors, vec![], "no vector stamped, none claimed");

        let last = expand_passage(&conn, &format!("{item_id}:000002"))
            .expect("expand")
            .expect("present");
        assert!(last.next.is_none(), "the last chunk has no next");
        assert!(last.previous.is_some());
    }

    #[test]
    fn expansion_of_unknown_chunk_is_none() {
        let (conn, _, _, _) = seed("exp-3");
        assert!(
            expand_passage(&conn, "missing:000000")
                .expect("expand")
                .is_none(),
            "unknown chunks answer None"
        );
    }

    #[test]
    fn expansion_skips_missing_pages_without_failing() {
        let (conn, item_id, _, middle) = seed("exp-4");
        conn.execute(
            "DELETE FROM bibliographic_page_texts WHERE attachment_id = 'att-exp' AND page_number = 2",
            [],
        )
        .expect("drop page two");
        let _ = item_id;
        let expansion: PassageExpansion = expand_passage(&conn, &middle)
            .expect("expand")
            .expect("present");
        assert_eq!(expansion.pages.len(), 1, "only the surviving page surfaces");
        assert_eq!(expansion.pages[0].page_number, 1);
        assert_eq!(expansion.pages[0].highlights, vec![(34, 60)]);
    }
}

// ── E4d-WU3: passage opening decisions (RED) ───────────────────────────────

#[cfg(test)]
mod open_tests {
    use super::super::attachment::plain_canonical;
    use super::super::retrieval::{prepare_passage_open, OriginalKind};

    fn open_db() -> rusqlite::Connection {
        // Reuses the expansion fixture shape: catalog, relations, profiles,
        // generations, chunks, embeddings, page texts.
        let conn = rusqlite::Connection::open_in_memory().expect("memory db");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0040_bibliography_catalog.sql"
        ))
        .expect("apply catalog foundation");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0041_bibliography_relations.sql"
        ))
        .expect("apply relations");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0047_bibliographic_semantic_profiles.sql"
        ))
        .expect("apply profiles table");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0049_bibliographic_index_generations.sql"
        ))
        .expect("apply generations");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0053_bibliographic_page_texts.sql"
        ))
        .expect("apply page texts");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0054_bibliographic_chunks.sql"
        ))
        .expect("apply chunks");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0055_bibliographic_chunk_embeddings.sql"
        ))
        .expect("apply chunk embeddings");
        conn
    }

    fn seed_chunk_with_attachment(
        conn: &mut rusqlite::Connection,
        native_path: Option<&str>,
    ) -> String {
        use super::super::repository::{
            upsert_attachment, upsert_connection, upsert_item, upsert_library, AttachmentInput,
            BibliographicItemInput, LibraryType, SourceOrigin, UpsertConnection, UpsertLibrary,
        };
        let source = upsert_connection(
            conn,
            UpsertConnection {
                id: "conn-1".to_string(),
                source_origin: SourceOrigin::Local,
                source_instance_id: None,
                endpoint: Some("http://synthetic.invalid".to_string()),
                capabilities_json: r#"{"read":true}"#.to_string(),
            },
        )
        .expect("connection");
        let library = upsert_library(
            conn,
            UpsertLibrary {
                connection_id: source.id,
                library_type: LibraryType::User,
                library_id: "0".to_string(),
                name: "Personal".to_string(),
                last_modified_version: Some(7),
            },
        )
        .expect("library");
        let item = upsert_item(
            conn,
            &library.id,
            BibliographicItemInput {
                item_key: "OPEN0001".to_string(),
                item_version: Some(3),
                native_json_snapshot: r#"{"key":"OPEN0001","version":3}"#.to_string(),
                csl_json_snapshot: r#"{"id":"OPEN0001","type":"book","title":"Obra abrible"}"#
                    .to_string(),
                title: Some("Obra abrible".to_string()),
                ..Default::default()
            },
        )
        .expect("catalog item");
        let attachment = upsert_attachment(
            conn,
            &item.id,
            AttachmentInput {
                attachment_key: "OPENATT1".to_string(),
                content_type: Some("application/pdf".to_string()),
                link_mode: Some("linked_file".to_string()),
                filename: Some("abrible.pdf".to_string()),
                native_path: native_path.map(String::from),
                url: None,
                md5: None,
                mtime: None,
                native_json_snapshot: r#"{"key":"OPENATT1"}"#.to_string(),
                native_version: None,
            },
        )
        .expect("catalog attachment");
        let chunk_id = format!("{}:000000", item.id);
        conn.execute(
            "INSERT INTO bibliographic_chunks
               (id, item_id, attachment_id, ordinal, text_content, text_hash,
                chunking_contract, created_at, updated_at)
             VALUES (?1, ?2, ?3, 0, 'Texto del fragmento abrible.', 'h-open', 'test-chunking', 1, 1)",
            rusqlite::params![chunk_id, item.id, attachment.id],
        )
        .expect("seed chunk");
        conn.execute(
            "INSERT INTO bibliographic_chunk_spans (chunk_id, page_number, start_char, end_char)
             VALUES (?1, 2, 10, 40)",
            [&chunk_id],
        )
        .expect("seed span");
        chunk_id
    }

    fn seed_typed(
        conn: &mut rusqlite::Connection,
        native_path: Option<&str>,
        content_type: &str,
        filename: &str,
    ) -> String {
        let chunk_id = seed_chunk_with_attachment(conn, native_path);
        conn.execute(
            "UPDATE zotero_attachments SET content_type = ?1, filename = ?2",
            rusqlite::params![content_type, filename],
        )
        .expect("retype attachment");
        chunk_id
    }

    #[test]
    fn open_plan_resolves_a_readable_pdf_beside_the_expansion() {
        let mut conn = open_db();
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = dir.path().join("abrible.pdf");
        std::fs::write(&pdf, b"%PDF-1.4 fake").expect("write pdf");
        let chunk_id = seed_chunk_with_attachment(&mut conn, Some(&pdf.to_string_lossy()));

        let plan = prepare_passage_open(&conn, &chunk_id, None).expect("prepare");
        assert_eq!(plan.expansion.chunk_id, chunk_id);
        assert_eq!(plan.expansion.spans, vec![(2, 10, 40)]);
        let original = plan.original.expect("a readable PDF is viewable");
        assert_eq!(original.kind, OriginalKind::Pdf);
        assert_eq!(
            original.path.as_deref(),
            Some(plain_canonical(&pdf).expect("canonical").as_path())
        );
        assert!(plan.reason.is_none(), "a resolved file carries no reason");
    }

    /// The owner's bug: Zotero stores `native_path` with forward slashes
    /// (`C:/Users/...`), and `explorer.exe C:/...` answers by opening the
    /// Documents folder. The path that leaves the plan is canonical and plain:
    /// no forward slashes on Windows, never the `\?\` verbatim form.
    #[test]
    fn open_plan_hands_out_a_canonical_native_path_not_the_stored_spelling() {
        let mut conn = open_db();
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = dir.path().join("abrible, con coma.pdf");
        std::fs::write(&pdf, b"%PDF-1.4 fake").expect("write pdf");
        let forward = pdf.to_string_lossy().replace('\\', "/");
        let chunk_id = seed_chunk_with_attachment(&mut conn, Some(&forward));

        let plan = prepare_passage_open(&conn, &chunk_id, None).expect("prepare");
        let path = plan
            .original
            .and_then(|original| original.path)
            .expect("the forward-slash path still resolves");
        let text = path.to_string_lossy().to_string();
        assert!(!text.starts_with(r"\\?\"), "verbatim prefix leaked: {text}");
        #[cfg(windows)]
        assert!(!text.contains('/'), "forward slashes survived: {text}");
    }

    #[test]
    fn open_plan_keeps_the_expansion_when_no_file_resolves() {
        let mut conn = open_db();
        let chunk_id = seed_chunk_with_attachment(&mut conn, None);

        let plan = prepare_passage_open(&conn, &chunk_id, None).expect("prepare");
        assert_eq!(plan.expansion.chunk_id, chunk_id);
        assert!(plan.original.is_none(), "nothing to open");
        let (reason, _detail) = plan.reason.expect("the reason travels with the context");
        assert_eq!(reason, "linked_file_missing");
    }

    #[test]
    fn open_plan_refuses_a_directory_as_the_original() {
        let mut conn = open_db();
        let dir = tempfile::tempdir().expect("tempdir");
        let chunk_id = seed_chunk_with_attachment(&mut conn, Some(&dir.path().to_string_lossy()));

        let plan = prepare_passage_open(&conn, &chunk_id, None).expect("prepare");
        assert!(plan.original.is_none(), "a directory is never an original");
        assert!(plan.reason.is_some(), "the refusal is explained");
    }

    #[test]
    fn open_plan_refuses_a_file_that_is_not_a_pdf() {
        let mut conn = open_db();
        let dir = tempfile::tempdir().expect("tempdir");
        let fake = dir.path().join("trampa.pdf");
        std::fs::write(&fake, b"MZ not a pdf at all").expect("write file");
        let chunk_id = seed_chunk_with_attachment(&mut conn, Some(&fake.to_string_lossy()));

        let plan = prepare_passage_open(&conn, &chunk_id, None).expect("prepare");
        assert!(plan.original.is_none(), "the PDF signature is checked");
        assert_eq!(plan.reason.expect("reason").0, "not_a_pdf");

        let docx = dir.path().join("notas.docx");
        std::fs::write(&docx, b"%PDF-1.4 but typed as word").expect("write docx");
        let mut other = open_db();
        let chunk = seed_typed(
            &mut other,
            Some(&docx.to_string_lossy()),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "notas.docx",
        );
        let plan = prepare_passage_open(&other, &chunk, None).expect("prepare");
        assert!(plan.original.is_none(), "the content type decides first");
        assert_eq!(plan.reason.expect("reason").0, "not_a_pdf");
    }

    /// Only a path derived from a registered attachment row is ever viewable:
    /// the command takes a chunk id, never a path, and a chunk whose
    /// attachment left the catalog yields nothing.
    #[test]
    fn open_plan_refuses_a_path_without_a_registered_attachment() {
        let mut conn = open_db();
        let dir = tempfile::tempdir().expect("tempdir");
        let pdf = dir.path().join("abrible.pdf");
        std::fs::write(&pdf, b"%PDF-1.4 fake").expect("write pdf");
        let chunk_id = seed_chunk_with_attachment(&mut conn, Some(&pdf.to_string_lossy()));
        // Foreign keys off: the chunk outlives its attachment row, the shape a
        // half-synced catalog can leave behind.
        conn.execute_batch("PRAGMA foreign_keys = OFF;")
            .expect("foreign keys off");
        conn.execute("DELETE FROM zotero_attachments", [])
            .expect("unregister the attachment");

        let plan = prepare_passage_open(&conn, &chunk_id, None).expect("prepare");
        assert!(plan.original.is_none());
        assert_eq!(plan.reason.expect("reason").0, "unknown_attachment");
    }

    /// An HTML snapshot opens from the catalog's stored text: no file is
    /// needed and none is ever granted.
    #[test]
    fn open_plan_views_an_html_snapshot_without_a_file() {
        let mut conn = open_db();
        let chunk_id = seed_typed(&mut conn, None, "text/html", "captura.html");

        let plan = prepare_passage_open(&conn, &chunk_id, None).expect("prepare");
        let original = plan.original.expect("a snapshot is viewable in app");
        assert_eq!(original.kind, OriginalKind::Html);
        assert!(original.path.is_none(), "no file is granted for HTML");
        assert!(plan.reason.is_none());
    }

    #[test]
    fn open_plan_for_unknown_chunks_fails_honestly() {
        let conn = open_db();
        let error = prepare_passage_open(&conn, "missing:000000", None)
            .expect_err("unknown chunks fail honestly");
        assert_eq!(error.code, "unknown_chunk");
    }
}

/// One synced library as the chat's scope control lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryStatusRow {
    pub library_type: String,
    pub library_native_id: String,
    pub name: String,
    /// Live works (tombstoned ones are not counted).
    pub works: i64,
    /// Chunks cut from this library's attachments.
    pub passages: i64,
}

/// Which libraries are synced and whether passage search can run at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryStatus {
    pub libraries: Vec<LibraryStatusRow>,
    /// An embedding generation is active for the effective contract. Without
    /// it passages cannot be ranked, however many chunks exist.
    pub vector_ready: bool,
}

/// Reads the catalog's synced libraries with their work and passage counts.
pub fn library_status(conn: &Connection, contract_hash: &str) -> BibliographyResult<LibraryStatus> {
    let mut statement = conn
        .prepare(
            "SELECT l.library_type, l.library_id, l.name,
                    (SELECT COUNT(*) FROM bibliographic_items i
                       LEFT JOIN zotero_item_tombstones t ON t.item_id = i.id
                      WHERE i.library_id = l.id AND t.item_id IS NULL),
                    (SELECT COUNT(*) FROM bibliographic_chunks c
                       JOIN bibliographic_items i ON i.id = c.item_id
                      WHERE i.library_id = l.id)
               FROM zotero_libraries l
              ORDER BY l.library_type, l.library_id",
        )
        .map_err(|error| err("Failed to prepare library status", error))?;
    let libraries = statement
        .query_map([], |row| {
            Ok(LibraryStatusRow {
                library_type: row.get(0)?,
                library_native_id: row.get(1)?,
                name: row.get(2)?,
                works: row.get(3)?,
                passages: row.get(4)?,
            })
        })
        .map_err(|error| err("Failed to read library status", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| err("Failed to collect library status", error))?;
    let vector_ready =
        crate::bibliography::generation::active_generation(conn, contract_hash)?.is_some();
    Ok(LibraryStatus {
        libraries,
        vector_ready,
    })
}

#[cfg(test)]
mod library_status_tests {
    use super::library_status;
    use super::passage_tests::{activate_gen, passage_db, seed_chunk, seed_item, CONTRACT};

    #[test]
    fn status_lists_synced_libraries_with_their_counts() {
        let mut conn = passage_db();
        let (_la, item_a) = seed_item(&mut conn, "pa", "a", "KA0001", "Obra A");
        let (_lb, _item_b) = seed_item(&mut conn, "pb", "b", "KB0001", "Obra B");
        let gen = activate_gen(&mut conn, "gen-status");
        seed_chunk(
            &conn,
            &item_a,
            0,
            "Texto de la obra A.",
            1,
            &gen,
            &[1.0, 0.0, 0.0, 0.0],
            None,
        );

        let status = library_status(&conn, CONTRACT).expect("status");
        assert!(status.vector_ready);
        let rows: Vec<_> = status
            .libraries
            .iter()
            .map(|row| {
                (
                    row.library_native_id.as_str(),
                    row.name.as_str(),
                    row.works,
                    row.passages,
                )
            })
            .collect();
        assert_eq!(
            rows,
            [("lib-a", "Personal a", 1, 1), ("lib-b", "Personal b", 1, 0)]
        );
        assert_eq!(status.libraries[0].library_type, "user");
    }

    #[test]
    fn status_without_libraries_or_generation_says_so() {
        let conn = passage_db();
        let status = library_status(&conn, CONTRACT).expect("status");
        assert!(status.libraries.is_empty());
        assert!(!status.vector_ready);
    }
}
