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
            "../../../../../packages/store/src/migrations/0038_bibliography_catalog.sql"
        ))
        .expect("apply catalog foundation");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0039_bibliography_relations.sql"
        ))
        .expect("apply relations");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0045_bibliographic_semantic_profiles.sql"
        ))
        .expect("apply profiles table");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0047_bibliographic_index_generations.sql"
        ))
        .expect("apply generations");
        // Test-only stub of the pre-0048 embeddings shape (see 0046): the
        // retrieval fixture never builds processing tables, so the 0046
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
        .expect("stub pre-0048 embeddings");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0048_bibliographic_embedding_generations.sql"
        ))
        .expect("apply embedding generations");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0049_bibliographic_profile_fts.sql"
        ))
        .expect("apply profile FTS");
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
        set_generation_manifest(&conn, &staging, floor, 11).expect("manifest");
        for _ in 0..floor {
            note_generation_progress(&conn, &staging).expect("progress");
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
    top_k.saturating_mul(5).max(20).min(500)
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
    let row: Option<(String, String, String, String, String)> = conn
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
    use std::collections::HashMap;
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
