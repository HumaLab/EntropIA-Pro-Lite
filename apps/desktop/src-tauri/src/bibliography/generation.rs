//! Bibliographic index generations (E3c-WU1): immutable contract rows plus a
//! staging/active/retired lifecycle with one active generation per contract.
//!
//! A generation builds vectors for exactly one vector space and only becomes
//! queryable when its manifest completes: the switch retires the previous
//! active generation and activates the staged one in a single transaction,
//! so a partial reindex is never presented as finished. Retiring the active
//! generation without a replacement is legitimate — retrieval then serves the
//! labeled lexical fallback until a new generation completes.
//!
//! Execution wiring (E3c-WU2) and hybrid retrieval (E3c-WU3) consume these
//! rows; this module only moves the pointer.

use rusqlite::{Connection, OptionalExtension as _};

use super::repository::{BibliographyError, BibliographyResult};

/// One immutable vector-space identity (plan §E3c contract row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingContractRow {
    pub contract_hash: String,
    pub provider: String,
    pub model: String,
    pub dimensions: i64,
    pub chunking_contract: String,
}

/// One index generation in any of its three states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexGeneration {
    pub id: String,
    pub contract_hash: String,
    pub status: String,
    pub expected_inputs: i64,
    pub completed_inputs: i64,
    pub created_at: i64,
    pub activated_at: Option<i64>,
    pub retired_at: Option<i64>,
}

pub(crate) fn read_generation(
    conn: &Connection,
    generation_id: &str,
) -> BibliographyResult<Option<IndexGeneration>> {
    let row = conn
        .query_row(
            "SELECT id, contract_hash, status, expected_inputs, completed_inputs,
                    created_at, activated_at, retired_at
             FROM bibliographic_index_generations WHERE id = ?1",
            [generation_id],
            |row| {
                Ok(IndexGeneration {
                    id: row.get(0)?,
                    contract_hash: row.get(1)?,
                    status: row.get(2)?,
                    expected_inputs: row.get(3)?,
                    completed_inputs: row.get(4)?,
                    created_at: row.get(5)?,
                    activated_at: row.get(6)?,
                    retired_at: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to read index generation {generation_id}: {error}"),
            )
        })?;
    Ok(row)
}

/// Registers one immutable contract row (insert-or-ignore): the same space
/// re-registers as the same row, never as a duplicate.
pub fn register_embedding_contract(
    conn: &Connection,
    contract: &EmbeddingContractRow,
    now_ms: i64,
) -> BibliographyResult<()> {
    conn.execute(
        "INSERT INTO bibliographic_embedding_contracts
           (contract_hash, provider, model, dimensions, chunking_contract, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(contract_hash) DO NOTHING",
        rusqlite::params![
            contract.contract_hash,
            contract.provider,
            contract.model,
            contract.dimensions,
            contract.chunking_contract,
            now_ms
        ],
    )
    .map_err(|error| {
        BibliographyError::new(
            "sql_error",
            format!("Failed to register embedding contract: {error}"),
        )
    })?;
    Ok(())
}

/// Opens (or attaches to the existing) staging generation for one contract.
/// Idempotent per contract: a second begin returns the same staging row
/// instead of orphaning parallel half-built generations.
pub fn begin_index_generation(
    conn: &Connection,
    contract_hash: &str,
    generation_id: &str,
    now_ms: i64,
) -> BibliographyResult<IndexGeneration> {
    let existing: Option<IndexGeneration> = conn
        .query_row(
            "SELECT id, contract_hash, status, expected_inputs, completed_inputs,
                    created_at, activated_at, retired_at
             FROM bibliographic_index_generations
             WHERE contract_hash = ?1 AND status = 'staging'
             ORDER BY created_at, id LIMIT 1",
            [contract_hash],
            |row| {
                Ok(IndexGeneration {
                    id: row.get(0)?,
                    contract_hash: row.get(1)?,
                    status: row.get(2)?,
                    expected_inputs: row.get(3)?,
                    completed_inputs: row.get(4)?,
                    created_at: row.get(5)?,
                    activated_at: row.get(6)?,
                    retired_at: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to check staging generations: {error}"),
            )
        })?;
    if let Some(generation) = existing {
        return Ok(generation);
    }
    conn.execute(
        "INSERT INTO bibliographic_index_generations
           (id, contract_hash, status, expected_inputs, completed_inputs, created_at)
         VALUES (?1, ?2, 'staging', 0, 0, ?3)",
        rusqlite::params![generation_id, contract_hash, now_ms],
    )
    .map_err(|error| {
        BibliographyError::new(
            "sql_error",
            format!("Failed to begin index generation: {error}"),
        )
    })?;
    read_generation(conn, generation_id)?
        .ok_or_else(|| BibliographyError::new("sql_error", "begin wrote no row"))
}

/// Declares the eligible-input manifest a generation must complete before
/// it may become active. Re-setting is allowed until activation (the corpus
/// grows between syncs); activation is what freezes the manifest.
pub fn set_generation_manifest(
    conn: &Connection,
    generation_id: &str,
    expected_inputs: i64,
    now_ms: i64,
) -> BibliographyResult<()> {
    if expected_inputs < 0 {
        return Err(BibliographyError::new(
            "invalid_input",
            "a generation manifest cannot be negative",
        ));
    }
    let changed = conn
        .execute(
            "UPDATE bibliographic_index_generations
             SET expected_inputs = ?1, created_at = created_at, completed_inputs = completed_inputs
             WHERE id = ?2 AND status = 'staging'",
            rusqlite::params![expected_inputs, generation_id],
        )
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to set generation manifest: {error}"),
            )
        })?;
    if changed == 0 {
        return Err(BibliographyError::new(
            "invalid_transition",
            format!("generation {generation_id} is not staging"),
        ));
    }
    let _ = now_ms;
    Ok(())
}

/// Records one completed input of a staging generation. Returns the new
/// completed count.
pub fn note_generation_progress(conn: &Connection, generation_id: &str) -> BibliographyResult<i64> {
    let changed = conn
        .execute(
            "UPDATE bibliographic_index_generations
             SET completed_inputs = completed_inputs + 1
             WHERE id = ?1 AND status = 'staging'",
            [generation_id],
        )
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to note generation progress: {error}"),
            )
        })?;
    if changed == 0 {
        return Err(BibliographyError::new(
            "invalid_transition",
            format!("generation {generation_id} is not staging"),
        ));
    }
    conn.query_row(
        "SELECT completed_inputs FROM bibliographic_index_generations WHERE id = ?1",
        [generation_id],
        |row| row.get(0),
    )
    .map_err(|error| {
        BibliographyError::new(
            "sql_error",
            format!("Failed to read generation progress: {error}"),
        )
    })
}

/// Activates a staging generation atomically: requires a non-empty complete
/// manifest, then retires the previous active generation of the same
/// contract and activates this one in one transaction. A partial generation
/// fails `generation_partial` and stays staging — it is never presented as
/// a finished reindex.
pub fn complete_index_generation(
    conn: &mut Connection,
    generation_id: &str,
    now_ms: i64,
) -> BibliographyResult<IndexGeneration> {
    let generation = read_generation(conn, generation_id)?.ok_or_else(|| {
        BibliographyError::new(
            "invalid_selection",
            format!("generation {generation_id} does not exist"),
        )
    })?;
    if generation.status != "staging" {
        return Err(BibliographyError::new(
            "invalid_transition",
            format!(
                "generation {generation_id} is {status}, not staging",
                status = generation.status
            ),
        ));
    }
    if generation.expected_inputs <= 0 || generation.completed_inputs < generation.expected_inputs {
        return Err(BibliographyError::new(
            "generation_partial",
            format!(
                "generation {generation_id} completed {} of {} expected inputs",
                generation.completed_inputs, generation.expected_inputs
            ),
        ));
    }
    let tx = conn
        .transaction()
        .map_err(|error| BibliographyError::new("sql_error", format!("{error}")))?;
    tx.execute(
        "UPDATE bibliographic_index_generations
         SET status = 'retired', retired_at = ?1
         WHERE contract_hash = ?2 AND status = 'active'",
        rusqlite::params![now_ms, generation.contract_hash],
    )
    .map_err(|error| {
        BibliographyError::new(
            "sql_error",
            format!("Failed to retire prior active generation: {error}"),
        )
    })?;
    tx.execute(
        "UPDATE bibliographic_index_generations
         SET status = 'active', activated_at = ?1
         WHERE id = ?2 AND status = 'staging'",
        rusqlite::params![now_ms, generation_id],
    )
    .map_err(|error| {
        BibliographyError::new(
            "sql_error",
            format!("Failed to activate generation: {error}"),
        )
    })?;
    tx.commit()
        .map_err(|error| BibliographyError::new("sql_error", format!("{error}")))?;
    read_generation(conn, generation_id)?
        .ok_or_else(|| BibliographyError::new("sql_error", "activation lost the row"))
}

/// Live works (not tombstoned) that carry a vector in one generation.
fn live_published_inputs(conn: &Connection, generation_id: &str) -> BibliographyResult<i64> {
    conn.query_row(
        "SELECT COUNT(DISTINCT e.item_id)
           FROM bibliographic_item_embeddings e
           JOIN bibliographic_items i ON i.id = e.item_id
           LEFT JOIN zotero_item_tombstones t ON t.item_id = e.item_id
          WHERE e.generation_id = ?1 AND t.item_id IS NULL",
        [generation_id],
        |row| row.get(0),
    )
    .map_err(|error| {
        BibliographyError::new(
            "sql_error",
            format!("Failed to count live published inputs: {error}"),
        )
    })
}

/// Live works that still owe this generation a vector and will deliver it: a
/// profile task of the generation's contract is pending, running, waiting to
/// retry or interrupted (a restart parks it for requeue), and no vector landed
/// yet. A deleted (tombstoned) work owes nothing. Neither does a work whose
/// profile ended `failed` or `cancelled` (or is `blocked` on configuration):
/// it will not finish by itself, so it must not hold the whole library's
/// search off. A later sync re-admits it and its vector folds into the active
/// generation.
fn outstanding_inputs(
    conn: &Connection,
    generation_id: &str,
    contract_hash: &str,
) -> BibliographyResult<i64> {
    conn.query_row(
        "SELECT COUNT(DISTINCT t.subject_id)
           FROM processing_tasks t
           JOIN bibliographic_items i ON i.id = t.subject_id
           LEFT JOIN zotero_item_tombstones z ON z.item_id = t.subject_id
          WHERE t.kind = 'bibliography_profile'
            AND t.domain = 'bibliography' AND t.subject_kind = 'item'
            AND t.contract_hash = ?2
            AND t.state IN ('pending', 'running', 'retry_wait', 'interrupted')
            AND z.item_id IS NULL
            AND NOT EXISTS (
                  SELECT 1 FROM bibliographic_item_embeddings e
                   WHERE e.generation_id = ?1 AND e.item_id = t.subject_id)",
        rusqlite::params![generation_id, contract_hash],
        |row| row.get(0),
    )
    .map_err(|error| {
        BibliographyError::new(
            "sql_error",
            format!("Failed to count outstanding inputs: {error}"),
        )
    })
}

/// Runs `body` inside a savepoint: it works both standalone and inside the
/// caller's transaction (the profile commit, the sync commit), and a failure
/// rolls back only its own writes.
fn in_savepoint<T>(
    conn: &Connection,
    body: impl FnOnce(&Connection) -> BibliographyResult<T>,
) -> BibliographyResult<T> {
    conn.execute_batch("SAVEPOINT generation_activation")
        .map_err(|error| BibliographyError::new("sql_error", format!("{error}")))?;
    match body(conn) {
        Ok(value) => {
            conn.execute_batch("RELEASE generation_activation")
                .map_err(|error| BibliographyError::new("sql_error", format!("{error}")))?;
            Ok(value)
        }
        Err(error) => {
            let _ = conn
                .execute_batch("ROLLBACK TO generation_activation; RELEASE generation_activation");
            Err(error)
        }
    }
}

/// Activates a staging generation once every work it owes has landed.
///
/// The manifest is re-derived from live data instead of trusting the stored
/// counters: `completed` is the number of live works that carry a vector in
/// this generation, `expected` is that plus the live works whose profile is
/// still in flight (pending, running, retry_wait, interrupted). Deleted works
/// and works whose profile failed or was cancelled are not owed, so neither
/// can strand the generation; work still in flight keeps it partial —
/// `Ok(None)`, still staging. Healed counters are
/// written back so the stored manifest never drifts from reality.
///
/// When the same contract already has an active generation, the staged
/// vectors are folded into it (an incremental sync only re-embeds the works
/// that changed, so replacing the active generation with the staging one
/// would drop every unchanged work); the staging row is retired. Otherwise
/// the staging row itself becomes active. Idempotent: a non-staging
/// generation is a no-op, so two commits finishing back to back cannot
/// activate twice. The caller supplies the surrounding transaction (or none).
pub fn activate_if_complete(
    conn: &Connection,
    generation_id: &str,
    now_ms: i64,
) -> BibliographyResult<Option<IndexGeneration>> {
    in_savepoint(conn, |conn| {
        let Some(generation) = read_generation(conn, generation_id)? else {
            return Ok(None);
        };
        if generation.status != "staging" {
            return Ok(None);
        }
        let completed = live_published_inputs(conn, generation_id)?;
        let outstanding = outstanding_inputs(conn, generation_id, &generation.contract_hash)?;
        let expected = completed + outstanding;
        if expected != generation.expected_inputs || completed != generation.completed_inputs {
            conn.execute(
                "UPDATE bibliographic_index_generations
                 SET expected_inputs = ?1, completed_inputs = ?2
                 WHERE id = ?3 AND status = 'staging'",
                rusqlite::params![expected, completed, generation_id],
            )
            .map_err(|error| {
                BibliographyError::new(
                    "sql_error",
                    format!("Failed to heal generation manifest: {error}"),
                )
            })?;
        }
        if completed == 0 || outstanding > 0 {
            return Ok(None);
        }
        let sql_error = |what: &'static str| {
            move |error: rusqlite::Error| {
                BibliographyError::new("sql_error", format!("Failed to {what}: {error}"))
            }
        };
        match active_generation(conn, &generation.contract_hash)? {
            None => {
                conn.execute(
                    "UPDATE bibliographic_index_generations
                     SET status = 'active', activated_at = ?1
                     WHERE id = ?2 AND status = 'staging'",
                    rusqlite::params![now_ms, generation_id],
                )
                .map_err(sql_error("activate generation"))?;
                read_generation(conn, generation_id)
            }
            Some(active) => {
                conn.execute(
                    "DELETE FROM bibliographic_item_embeddings
                      WHERE generation_id = ?1
                        AND item_id IN (SELECT item_id FROM bibliographic_item_embeddings
                                         WHERE generation_id = ?2)",
                    rusqlite::params![active.id, generation_id],
                )
                .map_err(sql_error("clear superseded work vectors"))?;
                conn.execute(
                    "UPDATE bibliographic_item_embeddings SET generation_id = ?1
                      WHERE generation_id = ?2",
                    rusqlite::params![active.id, generation_id],
                )
                .map_err(sql_error("fold work vectors into the active generation"))?;
                conn.execute(
                    "UPDATE OR REPLACE bibliographic_chunk_embeddings SET generation_id = ?1
                      WHERE generation_id = ?2",
                    rusqlite::params![active.id, generation_id],
                )
                .map_err(sql_error("fold passage vectors into the active generation"))?;
                conn.execute(
                    "UPDATE bibliographic_index_generations
                     SET status = 'retired', retired_at = ?1
                     WHERE id = ?2 AND status = 'staging'",
                    rusqlite::params![now_ms, generation_id],
                )
                .map_err(sql_error("retire the folded staging generation"))?;
                read_generation(conn, &active.id)
            }
        }
    })
}

/// Activates every complete staging generation (see [`activate_if_complete`]).
/// This is the repair for archives that completed a generation before any
/// production path activated it; it is cheap and safe to run at startup and
/// at the start of every sync publication. Returns how many became active.
pub fn activate_complete_staging_generations(
    conn: &Connection,
    now_ms: i64,
) -> BibliographyResult<usize> {
    let ids: Vec<String> = conn
        .prepare(
            "SELECT id FROM bibliographic_index_generations
             WHERE status = 'staging' ORDER BY created_at, id",
        )
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to list staging generations: {error}"),
            )
        })?;
    let mut activated = 0;
    for id in ids {
        if activate_if_complete(conn, &id, now_ms)?.is_some() {
            activated += 1;
        }
    }
    Ok(activated)
}

/// Retires a generation. Retiring the active one is legitimate: it leaves
/// no active space, and retrieval serves the labeled lexical fallback until
/// a new generation completes. A retired generation never reactivates.
pub fn retire_index_generation(
    conn: &Connection,
    generation_id: &str,
    now_ms: i64,
) -> BibliographyResult<()> {
    let changed = conn
        .execute(
            "UPDATE bibliographic_index_generations
             SET status = 'retired', retired_at = COALESCE(retired_at, ?1)
             WHERE id = ?2 AND status IN ('staging', 'active')",
            rusqlite::params![now_ms, generation_id],
        )
        .map_err(|error| {
            BibliographyError::new("sql_error", format!("Failed to retire generation: {error}"))
        })?;
    if changed == 0 {
        return Err(BibliographyError::new(
            "invalid_transition",
            format!("generation {generation_id} is already retired or missing"),
        ));
    }
    Ok(())
}

/// The active generation of one contract, if any — the only space
/// retrieval may query for that contract.
pub fn active_generation(
    conn: &Connection,
    contract_hash: &str,
) -> BibliographyResult<Option<IndexGeneration>> {
    let row = conn
        .query_row(
            "SELECT id, contract_hash, status, expected_inputs, completed_inputs,
                    created_at, activated_at, retired_at
             FROM bibliographic_index_generations
             WHERE contract_hash = ?1 AND status = 'active'",
            [contract_hash],
            |row| {
                Ok(IndexGeneration {
                    id: row.get(0)?,
                    contract_hash: row.get(1)?,
                    status: row.get(2)?,
                    expected_inputs: row.get(3)?,
                    completed_inputs: row.get(4)?,
                    created_at: row.get(5)?,
                    activated_at: row.get(6)?,
                    retired_at: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to read active generation: {error}"),
            )
        })?;
    Ok(row)
}

// ── E3c-WU2: execution wiring helpers ──────────────────────────────────────
//
/// Ensures a staging generation exists for one contract: registers the
/// contract row, then attaches to the existing staging row or begins a new
/// one. Idempotent — repeated calls share the same staging generation.
pub fn ensure_staging_generation_for_contract(
    conn: &Connection,
    contract: &EmbeddingContractRow,
    now_ms: i64,
) -> BibliographyResult<IndexGeneration> {
    register_embedding_contract(conn, contract, now_ms)?;
    let existing = conn
        .query_row(
            "SELECT id, contract_hash, status, expected_inputs, completed_inputs,
                    created_at, activated_at, retired_at
             FROM bibliographic_index_generations
             WHERE contract_hash = ?1 AND status = 'staging'
             ORDER BY created_at, id LIMIT 1",
            [&contract.contract_hash],
            |row| {
                Ok(IndexGeneration {
                    id: row.get(0)?,
                    contract_hash: row.get(1)?,
                    status: row.get(2)?,
                    expected_inputs: row.get(3)?,
                    completed_inputs: row.get(4)?,
                    created_at: row.get(5)?,
                    activated_at: row.get(6)?,
                    retired_at: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to check staging generations: {error}"),
            )
        })?;
    if let Some(generation) = existing {
        return Ok(generation);
    }
    let generation_id = format!("gen-{}", uuid::Uuid::new_v4());
    conn.execute(
        "INSERT INTO bibliographic_index_generations
           (id, contract_hash, status, expected_inputs, completed_inputs, created_at)
         VALUES (?1, ?2, 'staging', 0, 0, ?3)",
        rusqlite::params![generation_id, contract.contract_hash, now_ms],
    )
    .map_err(|error| {
        BibliographyError::new(
            "sql_error",
            format!("Failed to begin staging generation: {error}"),
        )
    })?;
    read_generation(conn, &generation_id)?
        .ok_or_else(|| BibliographyError::new("sql_error", "staging begin wrote no row"))
}

/// Raises a staging generation's manifest to at least `floor`, monotonic —
/// later chains can only grow the manifest, never shrink it below work
/// already admitted. Returns the resulting manifest.
pub fn raise_generation_manifest(
    conn: &Connection,
    generation_id: &str,
    floor: i64,
) -> BibliographyResult<i64> {
    let changed = conn
        .execute(
            "UPDATE bibliographic_index_generations
             SET expected_inputs = MAX(expected_inputs, ?1)
             WHERE id = ?2 AND status = 'staging'",
            rusqlite::params![floor, generation_id],
        )
        .map_err(|error| {
            BibliographyError::new(
                "sql_error",
                format!("Failed to raise generation manifest: {error}"),
            )
        })?;
    if changed == 0 {
        return Err(BibliographyError::new(
            "invalid_transition",
            format!("generation {generation_id} is not staging"),
        ));
    }
    conn.query_row(
        "SELECT expected_inputs FROM bibliographic_index_generations WHERE id = ?1",
        [generation_id],
        |row| row.get(0),
    )
    .map_err(|error| {
        BibliographyError::new(
            "sql_error",
            format!("Failed to read generation manifest: {error}"),
        )
    })
}

/// Counts distinct works with a vector stamped for one generation. Progress
/// is re-profiles-safe: re-publishing the same work updates its row in
/// place, so the count only grows when a new work lands.
pub fn generation_distinct_published(
    conn: &Connection,
    generation_id: &str,
) -> BibliographyResult<i64> {
    conn.query_row(
        "SELECT COUNT(DISTINCT item_id) FROM bibliographic_item_embeddings WHERE generation_id = ?1",
        [generation_id],
        |row| row.get(0),
    )
    .map_err(|error| {
        BibliographyError::new(
            "sql_error",
            format!("Failed to count generation progress: {error}"),
        )
    })
}

/// True when a work already carries a vector for one generation.
pub fn generation_has_item(
    conn: &Connection,
    generation_id: &str,
    item_id: &str,
) -> BibliographyResult<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM bibliographic_item_embeddings
         WHERE generation_id = ?1 AND item_id = ?2",
        rusqlite::params![generation_id, item_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
    .map_err(|error| {
        BibliographyError::new(
            "sql_error",
            format!("Failed to check generation item: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTRACT_A: &str = "contract-a";
    const CONTRACT_B: &str = "contract-b";

    fn generations_db() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0049_bibliographic_index_generations.sql"
        ))
        .expect("apply 0049 mirror");
        conn
    }

    fn contract(hash: &str) -> EmbeddingContractRow {
        EmbeddingContractRow {
            contract_hash: hash.to_string(),
            provider: "api".to_string(),
            model: "baai/bge-m3".to_string(),
            dimensions: 1024,
            chunking_contract: "rag-chunk-800-100-char-v1".to_string(),
        }
    }

    #[test]
    fn full_lifecycle_registers_once_stages_completes_and_switches_atomically() {
        let mut conn = generations_db();
        register_embedding_contract(&conn, &contract(CONTRACT_A), 1).expect("register");
        // Idempotent: the same space re-registers as the same row.
        register_embedding_contract(&conn, &contract(CONTRACT_A), 2).expect("re-register");
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bibliographic_embedding_contracts WHERE contract_hash = ?1",
                [CONTRACT_A],
                |row| row.get(0),
            )
            .expect("contract count");
        assert_eq!(rows, 1);

        let first = begin_index_generation(&conn, CONTRACT_A, "gen-1", 10).expect("begin");
        assert_eq!(first.status, "staging");
        // Second begin attaches to the same staging row.
        let attached = begin_index_generation(&conn, CONTRACT_A, "gen-2", 11).expect("attach");
        assert_eq!(attached.id, "gen-1", "no orphaned parallel staging");

        // A partial generation cannot activate.
        set_generation_manifest(&conn, "gen-1", 2, 12).expect("manifest");
        assert_eq!(
            note_generation_progress(&conn, "gen-1").expect("progress"),
            1
        );
        let error = complete_index_generation(&mut conn, "gen-1", 13)
            .expect_err("a partial generation must never activate");
        assert_eq!(error.code, "generation_partial");
        assert_eq!(
            read_generation(&conn, "gen-1")
                .expect("read")
                .expect("row")
                .status,
            "staging",
            "rejection keeps the generation staging"
        );

        // Complete the manifest: the switch activates and leaves one active.
        assert_eq!(
            note_generation_progress(&conn, "gen-1").expect("progress"),
            2
        );
        let active = complete_index_generation(&mut conn, "gen-1", 14).expect("activate");
        assert_eq!(active.status, "active");
        assert_eq!(active.activated_at, Some(14));
        assert_eq!(
            active_generation(&conn, CONTRACT_A)
                .expect("active read")
                .expect("row")
                .id,
            "gen-1"
        );

        // A new generation retires the old one exactly at its own switch.
        let _next = begin_index_generation(&conn, CONTRACT_A, "gen-3", 20).expect("begin");
        set_generation_manifest(&conn, "gen-3", 1, 21).expect("manifest");
        note_generation_progress(&conn, "gen-3").expect("progress");
        complete_index_generation(&mut conn, "gen-3", 22).expect("switch");
        assert_eq!(
            read_generation(&conn, "gen-1")
                .expect("read")
                .expect("row")
                .status,
            "retired",
            "the previous space retires exactly at the switch"
        );
        assert_eq!(
            read_generation(&conn, "gen-1")
                .expect("read")
                .expect("row")
                .retired_at,
            Some(22)
        );
        assert_eq!(
            active_generation(&conn, CONTRACT_A)
                .expect("read")
                .expect("row")
                .id,
            "gen-3"
        );
    }

    #[test]
    fn contracts_isolate_their_actives_and_retire_opens_the_lexical_fallback() {
        let mut conn = generations_db();
        register_embedding_contract(&conn, &contract(CONTRACT_A), 1).expect("register A");
        register_embedding_contract(&conn, &contract(CONTRACT_B), 1).expect("register B");
        for (contract_hash, generation_id) in [(CONTRACT_A, "gen-a"), (CONTRACT_B, "gen-b")] {
            begin_index_generation(&conn, contract_hash, generation_id, 10).expect("begin");
            set_generation_manifest(&conn, generation_id, 1, 11).expect("manifest");
            note_generation_progress(&conn, generation_id).expect("progress");
            complete_index_generation(&mut conn, generation_id, 12).expect("activate");
        }
        assert_eq!(
            active_generation(&conn, CONTRACT_A)
                .expect("read")
                .expect("row")
                .id,
            "gen-a"
        );
        assert_eq!(
            active_generation(&conn, CONTRACT_B)
                .expect("read")
                .expect("row")
                .id,
            "gen-b",
            "each contract keeps its own queryable space"
        );

        // Retiring the active space with no replacement is legitimate: the
        // query falls back to labeled lexical results until a new space
        // completes.
        retire_index_generation(&conn, "gen-a", 13).expect("retire active");
        assert!(
            active_generation(&conn, CONTRACT_A)
                .expect("read")
                .is_none(),
            "no active space remains for A"
        );
        assert!(
            active_generation(&conn, CONTRACT_B)
                .expect("read")
                .is_some(),
            "the other contract is untouched"
        );
        // A retired generation never reactivates and rejects progress.
        assert!(
            note_generation_progress(&conn, "gen-a").is_err(),
            "retired generations accept no progress"
        );
        assert!(
            complete_index_generation(&mut conn, "gen-a", 14).is_err(),
            "retired generations never reactivate"
        );
    }

    #[test]
    fn zero_manifest_or_unknown_generations_fail_honestly() {
        let mut conn = generations_db();
        register_embedding_contract(&conn, &contract(CONTRACT_A), 1).expect("register");
        begin_index_generation(&conn, CONTRACT_A, "gen-1", 10).expect("begin");
        // No manifest declared: expected stays 0 and activation refuses.
        let error = complete_index_generation(&mut conn, "gen-1", 11)
            .expect_err("an unmanifested generation must not activate");
        assert_eq!(error.code, "generation_partial");
        assert!(
            complete_index_generation(&mut conn, "gen-missing", 12).is_err(),
            "unknown generations fail honestly"
        );
        assert!(
            retire_index_generation(&conn, "gen-missing", 13).is_err(),
            "retiring the unknown fails honestly"
        );
    }
}
