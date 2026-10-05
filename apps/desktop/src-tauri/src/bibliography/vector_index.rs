//! In-memory vector index of the bibliography passage search (vector leg).
//!
//! Scanning every chunk vector of the active generation straight from SQLite
//! costs seconds per query (each vector is a 4 KiB blob). The index keeps one
//! int8 copy of them (~1 KiB per 1024-dimension vector, see the `vecscan`
//! crate) built once per (archive, generation) and rebuilt when the chunk
//! vectors change. A query scans the matrix for a shortlist and re-ranks it
//! exactly against the original f32 vectors in SQLite, so the returned scores
//! and order are those of an exact scan whenever the true top-k is inside the
//! shortlist (the shortlist is [`SHORTLIST_FACTOR`] times the depth).
//!
//! The first query after a change pays the build (a single pass over the
//! blobs); the lexical index follows the same lazy, fingerprint-keyed scheme.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use vecscan::QuantMatrix;

use super::repository::{BibliographyError, BibliographyResult};

/// Shortlist size per requested result: the quantized scan nominates this
/// many times the depth and the exact re-rank keeps the best `depth`.
const SHORTLIST_FACTOR: usize = 4;

fn err(context: &str, error: impl std::fmt::Display) -> BibliographyError {
    BibliographyError::new("vector_index_failed", format!("{context}: {error}"))
}

/// The chunk vectors of one generation, quantized, with the ids and works
/// they belong to.
pub struct VectorIndex {
    matrix: QuantMatrix,
    chunk_ids: Vec<String>,
    /// Works of the indexed chunks, by the slot each row carries as its group.
    items: Vec<String>,
}

impl VectorIndex {
    /// Number of indexed chunk vectors.
    pub fn len(&self) -> usize {
        self.chunk_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunk_ids.is_empty()
    }

    /// Dimensions of the indexed vectors (0 when empty).
    pub fn dims(&self) -> usize {
        self.matrix.dims()
    }

    /// Approximate heap size, for diagnostics and the memory budget.
    pub fn approx_bytes(&self) -> usize {
        self.matrix.heap_bytes()
            + self
                .chunk_ids
                .iter()
                .map(|id| id.capacity() + std::mem::size_of::<String>())
                .sum::<usize>()
            + self
                .items
                .iter()
                .map(|item| item.capacity() + std::mem::size_of::<String>())
                .sum::<usize>()
    }
}

/// Reads every fresh chunk vector of the generation into a new index. Rows
/// whose blob has another dimension than the first one, or that hold a zero
/// or non-finite vector, are left out (they could not be scored anyway).
fn build(conn: &Connection, generation_id: &str) -> BibliographyResult<VectorIndex> {
    let expected: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM bibliographic_chunk_embeddings WHERE generation_id = ?1",
            [generation_id],
            |row| row.get(0),
        )
        .map_err(|error| err("Failed to count chunk vectors", error))?;
    let expected = usize::try_from(expected).unwrap_or(0);
    let mut stmt = conn
        .prepare(
            "SELECT e.chunk_id, c.item_id, e.embedding
             FROM bibliographic_chunk_embeddings e
             JOIN bibliographic_chunks c ON c.id = e.chunk_id
             LEFT JOIN zotero_item_tombstones t ON t.item_id = c.item_id
             WHERE e.generation_id = ?1 AND t.item_id IS NULL
               AND e.input_hash = c.text_hash",
        )
        .map_err(|error| err("Failed to prepare the vector index", error))?;
    let mut rows = stmt
        .query([generation_id])
        .map_err(|error| err("Failed to read chunk vectors", error))?;
    let mut matrix: Option<QuantMatrix> = None;
    let mut chunk_ids: Vec<String> = Vec::with_capacity(expected);
    let mut items: Vec<String> = Vec::new();
    let mut slots: HashMap<String, u32> = HashMap::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| err("Failed to read chunk vectors", error))?
    {
        let blob = row
            .get_ref(2)
            .and_then(|value| value.as_blob().map_err(Into::into))
            .map_err(|error| err("Failed to read a chunk vector", error))?;
        if blob.is_empty() || blob.len() % 4 != 0 {
            continue;
        }
        let item_id = row
            .get_ref(1)
            .and_then(|value| value.as_str().map_err(Into::into))
            .map_err(|error| err("Failed to read a chunk work", error))?;
        let slot = match slots.get(item_id) {
            Some(slot) => *slot,
            None => {
                let slot = u32::try_from(items.len())
                    .map_err(|error| err("Too many works to index", error))?;
                slots.insert(item_id.to_string(), slot);
                items.push(item_id.to_string());
                slot
            }
        };
        let matrix =
            matrix.get_or_insert_with(|| QuantMatrix::with_capacity(blob.len() / 4, expected));
        if !matrix.push_blob(blob, slot) {
            continue;
        }
        let chunk_id: String = row
            .get(0)
            .map_err(|error| err("Failed to read a chunk id", error))?;
        chunk_ids.push(chunk_id);
    }
    chunk_ids.shrink_to_fit();
    Ok(VectorIndex {
        matrix: matrix.unwrap_or_else(|| QuantMatrix::new(0)),
        chunk_ids,
        items,
    })
}

/// What decides when the index is stale: the archive file, the generation,
/// and a fingerprint of the vectors (count, highest row id and a checksum of
/// their input hashes, which changes when a vector is re-made in place) plus
/// the tombstones. Read from covering indexes: no blob is touched.
fn cache_key(conn: &Connection, generation_id: &str) -> BibliographyResult<Option<String>> {
    let Some(path) = conn.path().filter(|path| !path.is_empty()) else {
        // An in-memory archive has no identity to key on: never cached.
        return Ok(None);
    };
    let (count, last, checksum): (i64, i64, f64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(MAX(rowid), 0),
                    TOTAL(unicode(substr(input_hash, 1, 1))
                          + 131 * unicode(substr(input_hash, 2, 1))
                          + 17161 * unicode(substr(input_hash, 3, 1))
                          + 2248091 * unicode(substr(input_hash, 4, 1)))
             FROM bibliographic_chunk_embeddings",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| err("Failed to fingerprint chunk vectors", error))?;
    let tombstones: i64 = conn
        .query_row("SELECT COUNT(*) FROM zotero_item_tombstones", [], |row| {
            row.get(0)
        })
        .map_err(|error| err("Failed to fingerprint tombstones", error))?;
    Ok(Some(format!(
        "{path}|{generation_id}|{count}|{last}|{checksum}|{tombstones}"
    )))
}

/// The index of the archive and generation, built on first use. One slot:
/// only the active generation of the open archive is ever queried. The lock
/// is held while building so concurrent first queries build it once.
pub fn index_for(conn: &Connection, generation_id: &str) -> BibliographyResult<Arc<VectorIndex>> {
    static CACHE: Mutex<Option<(String, Arc<VectorIndex>)>> = Mutex::new(None);

    let Some(key) = cache_key(conn, generation_id)? else {
        return Ok(Arc::new(build(conn, generation_id)?));
    };
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((cached_key, index)) = cache.as_ref() {
        if *cached_key == key {
            return Ok(Arc::clone(index));
        }
    }
    // Drop the stale index before allocating its replacement.
    *cache = None;
    let index = Arc::new(build(conn, generation_id)?);
    *cache = Some((key, Arc::clone(&index)));
    Ok(index)
}

/// Builds (or confirms) the index ahead of the first query.
pub fn warm(conn: &Connection, generation_id: &str) -> BibliographyResult<()> {
    index_for(conn, generation_id).map(|_| ())
}

/// Vector leg: the `depth` chunks of the generation closest to the query by
/// cosine similarity, best first (ties to the lower chunk id). `allowed`
/// restricts the works. The shortlist comes from the quantized matrix and is
/// re-ranked exactly; a chunk whose vector went stale or whose work was
/// tombstoned since the index was built is dropped at that step.
pub fn search(
    conn: &Connection,
    generation_id: &str,
    query_vector: &[f32],
    allowed: Option<&HashSet<String>>,
    depth: usize,
) -> BibliographyResult<Vec<(String, f64)>> {
    let Some(unit_query) = vecscan::unit(query_vector) else {
        return Ok(Vec::new());
    };
    if depth == 0 {
        return Ok(Vec::new());
    }
    let index = index_for(conn, generation_id)?;
    if index.is_empty() || index.matrix.dims() != unit_query.len() {
        return Ok(Vec::new());
    }
    let allowed_slots: Option<Vec<bool>> =
        allowed.map(|set| index.items.iter().map(|item| set.contains(item)).collect());
    let shortlist = index.matrix.scan(
        &unit_query,
        depth.saturating_mul(SHORTLIST_FACTOR),
        allowed_slots.as_deref(),
    );
    let mut stmt = conn
        .prepare(
            "SELECT e.embedding
             FROM bibliographic_chunk_embeddings e
             JOIN bibliographic_chunks c ON c.id = e.chunk_id
             WHERE e.chunk_id = ?1 AND e.generation_id = ?2
               AND e.input_hash = c.text_hash
               AND NOT EXISTS (SELECT 1 FROM zotero_item_tombstones t WHERE t.item_id = c.item_id)",
        )
        .map_err(|error| err("Failed to prepare the vector re-rank", error))?;
    let mut best: Vec<(String, f64)> = Vec::with_capacity(shortlist.len());
    for (row, _approximate) in shortlist {
        let chunk_id = &index.chunk_ids[row as usize];
        let mut found = stmt
            .query(rusqlite::params![chunk_id, generation_id])
            .map_err(|error| err("Failed to re-rank a chunk vector", error))?;
        let Some(found) = found
            .next()
            .map_err(|error| err("Failed to re-rank a chunk vector", error))?
        else {
            continue;
        };
        let blob = found
            .get_ref(0)
            .and_then(|value| value.as_blob().map_err(Into::into))
            .map_err(|error| err("Failed to read a chunk vector", error))?;
        if let Some(similarity) = vecscan::unit_similarity_blob(&unit_query, blob) {
            best.push((chunk_id.clone(), f64::from(similarity)));
        }
    }
    best.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    best.truncate(depth);
    Ok(best)
}
