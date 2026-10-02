//! Composed retrieval (E7a): corpus-only, bibliography-only, and combined
//! search with separate budgets and per-domain provenance.
//!
//! The two domains never share a ranking: corpus candidates come from the
//! documentary RRF pipeline, bibliography hits from the work-level hybrid
//! search, each under its own budget. The answer carries both lists with
//! the space identity of each side, so the UI renders sections — never a
//! fused cross-space leaderboard. An embed failure degrades each leg to
//! its lexical fallback instead of failing the query.

use rusqlite::Connection;

use super::retrieval::{HybridQuery, WorkFilters, WorkHit};

/// Which domains one composed query covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainSelection {
    pub corpus: bool,
    pub bibliography: bool,
}

/// One composed query: shared text, per-domain budgets.
#[derive(Debug, Clone)]
pub struct ComposedQuery {
    pub text: String,
    pub domains: DomainSelection,
    pub corpus_top_k: usize,
    pub bibliography_top_k: usize,
    pub bibliography_filters: WorkFilters,
}

/// One documentary hit with its collection provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct CorpusHit {
    pub asset_id: String,
    pub item_id: String,
    pub title: String,
    pub collection_id: String,
    pub collection_name: String,
    pub score: f64,
    /// False when the query ran without an embedding (lexical fallback).
    pub embedded: bool,
}

/// A composed answer: two budgeted lists, each labeled with its space.
#[derive(Debug, Clone, PartialEq)]
pub struct ComposedAnswer {
    pub corpus: Vec<CorpusHit>,
    pub bibliography: Vec<WorkHit>,
    pub corpus_embedded: bool,
    pub bibliography_vector_available: bool,
    pub bibliography_contract: Option<String>,
    pub bibliography_generation: Option<String>,
}

/// Runs one composed query: each selected domain under its own budget,
/// each answer labeled with its provenance.
pub fn compose_search(
    conn: &Connection,
    query: &ComposedQuery,
    embed_query: &dyn Fn(&str) -> Result<Vec<f32>, String>,
) -> Result<ComposedAnswer, String> {
    use crate::rag::params::RagParams;
    use crate::rag::retrieval::{hybrid_retrieve_candidates, RetrievalQuery, RetrievalUnit};
    let empty = ComposedAnswer {
        corpus: Vec::new(),
        bibliography: Vec::new(),
        corpus_embedded: false,
        bibliography_vector_available: false,
        bibliography_contract: None,
        bibliography_generation: None,
    };
    if query.text.trim().is_empty() {
        return Ok(empty);
    }
    // Documentary leg under its own budget. An embed failure degrades to
    // the lexical leg instead of failing the query.
    let mut corpus = Vec::new();
    let mut corpus_embedded = false;
    if query.domains.corpus {
        let top_k = query.corpus_top_k.max(1);
        let mut params = RagParams::default();
        params.top_k = top_k;
        params.candidates_per_leg = (top_k.saturating_mul(2)).max(10);
        params.fusion_candidate_limit = params.candidates_per_leg.saturating_mul(2);
        params.rerank_depth = top_k;
        let embedding = embed_query(&query.text).ok();
        corpus_embedded = embedding.is_some();
        let queries = [RetrievalQuery {
            text: query.text.as_str(),
            embedding: embedding.as_deref(),
        }];
        let candidates = hybrid_retrieve_candidates(conn, &queries, &params, RetrievalUnit::Asset)?;
        corpus = candidates
            .into_iter()
            .take(top_k)
            .map(|candidate| CorpusHit {
                asset_id: candidate.record.asset_id.clone(),
                item_id: candidate.record.item_id.clone(),
                title: candidate.record.item_title.clone(),
                collection_id: candidate.record.collection_id.clone(),
                collection_name: candidate.record.collection_name.clone(),
                score: candidate.score,
                embedded: corpus_embedded,
            })
            .collect();
    }
    // Bibliographic leg under its own budget and filters. The query
    // contract resolves from settings; an unknown provider is a
    // misconfiguration worth failing loudly.
    let mut bibliography = Vec::new();
    let mut bibliography_vector_available = false;
    let mut bibliography_contract = None;
    let mut bibliography_generation = None;
    if query.domains.bibliography {
        let effective = crate::processing::eligibility::resolve_effective_embedding_contract(conn)?;
        let answer = super::retrieval::search_works(
            conn,
            &effective.hash,
            &HybridQuery {
                text: query.text.clone(),
                top_k: query.bibliography_top_k.max(1),
                filters: query.bibliography_filters.clone(),
            },
            embed_query,
        )
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
        bibliography = answer.hits;
        bibliography_vector_available = answer.vector_available;
        bibliography_contract = Some(answer.contract_hash);
        bibliography_generation = answer.active_generation_id;
    }
    Ok(ComposedAnswer {
        corpus,
        bibliography,
        corpus_embedded,
        bibliography_vector_available,
        bibliography_contract,
        bibliography_generation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floats_to_blob(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    fn big_vector(first: f32, second: f32) -> Vec<f32> {
        let mut vector = vec![0.0f32; 1024];
        vector[0] = first;
        vector[1] = second;
        vector
    }

    fn composed_db() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        // Documentary side (minimal RAG shape).
        conn.execute_batch(
            "CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE items (id TEXT PRIMARY KEY, collection_id TEXT, title TEXT NOT NULL, metadata TEXT);
             CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL, type TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE transcriptions (id TEXT PRIMARY KEY, asset_id TEXT UNIQUE, text_content TEXT NOT NULL, language TEXT, duration_ms INTEGER, model TEXT, segments TEXT, confidence REAL, created_at INTEGER NOT NULL);
             CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT NOT NULL, method TEXT, confidence REAL, created_at INTEGER NOT NULL);
             CREATE TABLE vec_assets (asset_id TEXT PRIMARY KEY, item_id TEXT NOT NULL, embedding BLOB NOT NULL, embedding_model TEXT NOT NULL DEFAULT 'legacy', embedding_contract TEXT NOT NULL DEFAULT 'legacy', dimensions INTEGER NOT NULL DEFAULT 0);",
        )
        .expect("corpus tables");
        conn.execute_batch(crate::nlp::fts::FTS_ITEMS_DDL)
            .expect("corpus FTS");
        // Bibliography side (real migrations).
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
        // composition fixture never builds processing tables, so the 0046
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

    fn seed_corpus_doc(
        conn: &Connection,
        asset_id: &str,
        title: &str,
        text: &str,
        vector: Vec<f32>,
    ) {
        conn.execute(
            "INSERT OR IGNORE INTO collections(id, name) VALUES ('c1', 'Legajo')",
            [],
        )
        .expect("collection insert");
        conn.execute(
            "INSERT INTO items(id, collection_id, title, metadata) VALUES (?1, 'c1', ?2, '{}')",
            rusqlite::params![format!("item-{asset_id}"), title],
        )
        .expect("item insert");
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, created_at) VALUES (?1, ?2, 'doc.pdf', 'pdf', 1)",
            rusqlite::params![asset_id, format!("item-{asset_id}")],
        )
        .expect("asset insert");
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, method, confidence, created_at)
             VALUES (?1, ?2, ?3, 'ocr', 0.9, 1)",
            rusqlite::params![format!("ext-{asset_id}"), asset_id, text],
        )
        .expect("extraction insert");
        conn.execute(
            "INSERT INTO vec_assets(asset_id, item_id, embedding, embedding_model, embedding_contract, dimensions)
             VALUES (?1, ?2, ?3, 'baai/bge-m3', 'bge-m3-6000-char-weighted-mean-l2-v1', 1024)",
            rusqlite::params![asset_id, format!("item-{asset_id}"), floats_to_blob(&vector)],
        )
        .expect("embedding insert");
        crate::nlp::fts::fts_index_item(conn, &format!("item-{asset_id}"), title, "", text)
            .expect("fts index");
    }

    fn seed_biblio_work(
        conn: &mut Connection,
        item_key: &str,
        title: &str,
        text: &str,
        vector: Vec<f32>,
    ) -> String {
        use super::super::generation::{
            begin_index_generation, complete_index_generation, note_generation_progress,
            register_embedding_contract, set_generation_manifest, EmbeddingContractRow,
        };
        use super::super::repository::{
            upsert_connection, upsert_item, upsert_library, BibliographicItemInput, LibraryType,
            SourceOrigin, UpsertConnection, UpsertLibrary,
        };
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
                library_id: "0".to_string(),
                name: format!("Personal {item_key}"),
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
                    "issued": { "date-parts": [[2021]] },
                })
                .to_string(),
                title: Some(title.to_string()),
                ..Default::default()
            },
        )
        .expect("catalog item");
        let hash = super::super::profile::profile_input_hash(text);
        conn.execute(
            "INSERT INTO bibliographic_semantic_profiles
               (item_id, profile_revision, template_version, canonical_text,
                input_hash, field_provenance_json, created_at, updated_at)
             VALUES (?1, 1, 'bibliography-profile-v1', ?2, ?3, '[]', 1, 1)",
            rusqlite::params![item.id, text, hash],
        )
        .expect("seed profile");
        // Canonical effective contract (no settings overrides in tests).
        let effective = crate::processing::eligibility::resolve_effective_embedding_contract(conn)
            .expect("effective contract");
        register_embedding_contract(
            conn,
            &EmbeddingContractRow {
                contract_hash: effective.hash.clone(),
                provider: effective.provider.clone(),
                model: effective.model.clone(),
                dimensions: effective.dimensions as i64,
                chunking_contract: "test-chunking".to_string(),
            },
            1,
        )
        .expect("register contract");
        let staging = begin_index_generation(conn, &effective.hash, &format!("gen-{item_key}"), 10)
            .expect("begin");
        let staging_id: String = conn
            .query_row(
                "SELECT id FROM bibliographic_index_generations
                 WHERE contract_hash = ?1 AND status = 'staging'",
                [&effective.hash],
                |row| row.get(0),
            )
            .expect("staging id");
        let _ = staging;
        set_generation_manifest(&conn, &staging_id, 1, 11).expect("manifest");
        note_generation_progress(&conn, &staging_id).expect("progress");
        complete_index_generation(conn, &staging_id, 20).expect("activate");
        conn.execute(
            "INSERT INTO bibliographic_item_embeddings
               (item_id, generation_id, embedding_contract, embedding_model,
                dimensions, embedding, input_hash, profile_revision, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1024, ?5, ?6, 1, 1, 1)",
            rusqlite::params![
                item.id,
                staging_id,
                effective.hash,
                effective.model,
                floats_to_blob(&vector),
                hash
            ],
        )
        .expect("seed work vector");
        item.id
    }

    fn combined_query(top_corpus: usize, top_biblio: usize) -> ComposedQuery {
        ComposedQuery {
            text: "revoluciones".to_string(),
            domains: DomainSelection {
                corpus: true,
                bibliography: true,
            },
            corpus_top_k: top_corpus,
            bibliography_top_k: top_biblio,
            bibliography_filters: WorkFilters::default(),
        }
    }

    #[test]
    fn domains_gate_each_leg() {
        let mut conn = composed_db();
        seed_corpus_doc(
            &conn,
            "a1",
            "Documento corpus",
            "texto sobre revoluciones agrarias",
            big_vector(1.0, 0.0),
        );
        seed_biblio_work(
            &mut conn,
            "DOMA0001",
            "Obra biblio",
            "Título: Obra biblio sobre revoluciones.",
            big_vector(1.0, 0.0),
        );
        let embed = |_: &str| Ok(big_vector(1.0, 0.0));

        let corpus_only = compose_search(
            &conn,
            &ComposedQuery {
                domains: DomainSelection {
                    corpus: true,
                    bibliography: false,
                },
                ..combined_query(5, 5)
            },
            &embed,
        )
        .expect("compose");
        assert_eq!(corpus_only.corpus.len(), 1, "corpus leg runs");
        assert!(
            corpus_only.bibliography.is_empty(),
            "bibliography leg stays off"
        );
        assert!(corpus_only.bibliography_contract.is_none());

        let biblio_only = compose_search(
            &conn,
            &ComposedQuery {
                domains: DomainSelection {
                    corpus: false,
                    bibliography: true,
                },
                ..combined_query(5, 5)
            },
            &embed,
        )
        .expect("compose");
        assert!(biblio_only.corpus.is_empty(), "corpus leg stays off");
        assert_eq!(biblio_only.bibliography.len(), 1, "bibliography leg runs");
        assert!(biblio_only.bibliography_contract.is_some());

        let neither = compose_search(
            &conn,
            &ComposedQuery {
                domains: DomainSelection {
                    corpus: false,
                    bibliography: false,
                },
                ..combined_query(5, 5)
            },
            &embed,
        )
        .expect("compose");
        assert!(neither.corpus.is_empty() && neither.bibliography.is_empty());
    }

    #[test]
    fn budgets_apply_per_domain() {
        let mut conn = composed_db();
        seed_corpus_doc(
            &conn,
            "a1",
            "Doc uno",
            "revoluciones uno",
            big_vector(1.0, 0.0),
        );
        seed_corpus_doc(
            &conn,
            "a2",
            "Doc dos",
            "revoluciones dos",
            big_vector(1.0, 0.0),
        );
        for (key, title) in [
            ("BUD00001", "Obra uno"),
            ("BUD00002", "Obra dos"),
            ("BUD00003", "Obra tres"),
        ] {
            seed_biblio_work(
                &mut conn,
                key,
                title,
                &format!("Título: {title} sobre revoluciones."),
                big_vector(1.0, 0.0),
            );
        }
        let embed = |_: &str| Ok(big_vector(1.0, 0.0));

        let answer = compose_search(&conn, &combined_query(1, 2), &embed).expect("compose");
        assert_eq!(answer.corpus.len(), 1, "corpus budget binds the corpus leg");
        assert_eq!(
            answer.bibliography.len(),
            2,
            "bibliography budget binds its own leg"
        );
    }

    #[test]
    fn embed_failure_degrades_both_legs_to_lexical() {
        let mut conn = composed_db();
        seed_corpus_doc(
            &conn,
            "a1",
            "Documento corpus",
            "texto sobre revoluciones agrarias",
            big_vector(1.0, 0.0),
        );
        seed_biblio_work(
            &mut conn,
            "DEG00001",
            "Obra biblio",
            "Título: Obra biblio sobre revoluciones.",
            big_vector(1.0, 0.0),
        );
        let failing = |_: &str| -> Result<Vec<f32>, String> { Err("no engine".to_string()) };

        let answer = compose_search(&conn, &combined_query(5, 5), &failing).expect("compose");
        assert!(!answer.corpus_embedded, "corpus runs lexical-only");
        assert!(
            !answer.bibliography_vector_available,
            "bibliography runs lexical-only"
        );
        assert_eq!(answer.corpus.len(), 1, "lexical corpus hits still surface");
        assert_eq!(
            answer.bibliography.len(),
            1,
            "lexical bibliography hits still surface"
        );
        assert!(answer
            .bibliography
            .iter()
            .all(|hit| hit.method == "lexical"));
    }

    #[test]
    fn provenance_names_domains_and_spaces() {
        let mut conn = composed_db();
        seed_corpus_doc(
            &conn,
            "a1",
            "Documento corpus",
            "texto sobre revoluciones agrarias",
            big_vector(1.0, 0.0),
        );
        seed_biblio_work(
            &mut conn,
            "PRV00001",
            "Obra biblio",
            "Título: Obra biblio sobre revoluciones.",
            big_vector(1.0, 0.0),
        );
        let embed = |_: &str| Ok(big_vector(1.0, 0.0));

        let answer = compose_search(&conn, &combined_query(5, 5), &embed).expect("compose");
        let corpus_hit = &answer.corpus[0];
        assert_eq!(corpus_hit.asset_id, "a1");
        assert!(corpus_hit.embedded, "vectors flowed on the corpus leg");
        assert!(corpus_hit.score > 0.0);
        let biblio_hit = answer
            .bibliography
            .iter()
            .find(|hit| hit.method != "lexical")
            .expect("a vector biblio hit");
        assert!(biblio_hit.contract_hash.is_some() && biblio_hit.generation_id.is_some());
        assert_eq!(
            answer.bibliography_contract.as_deref(),
            biblio_hit.contract_hash.as_deref()
        );
    }
}
