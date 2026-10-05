//! Quantized in-memory cosine scan.
//!
//! The matrix keeps every vector as int8 (one scale per row, the norm folded
//! in), so a 1024-dimension vector costs ~1 KiB instead of 4 KiB and a scan is
//! an integer dot product the compiler vectorizes. Scores are approximations
//! meant to nominate a shortlist: callers re-rank it exactly against the
//! original f32 vectors (see [`unit_similarity_blob`]).
//!
//! This crate is deliberately tiny and non-generic: the app compiles it at
//! `opt-level = 3` even in debug builds (`[profile.dev.package.vecscan]`), and
//! generic code would be instantiated in the caller at the caller's level.

/// Quantized matrix of vectors, one `group` tag per row.
pub struct QuantMatrix {
    dims: usize,
    data: Vec<i8>,
    /// Per row: `max_abs / 127 / norm`, so `unit value ~= q * scale`.
    scales: Vec<f32>,
    groups: Vec<u32>,
    scratch: Vec<f32>,
}

/// A quantized query.
struct QuantQuery {
    q: Vec<i8>,
    scale: f32,
}

impl QuantMatrix {
    pub fn new(dims: usize) -> Self {
        Self::with_capacity(dims, 0)
    }

    pub fn with_capacity(dims: usize, rows: usize) -> Self {
        Self {
            dims,
            data: Vec::with_capacity(dims * rows),
            scales: Vec::with_capacity(rows),
            groups: Vec::with_capacity(rows),
            scratch: Vec::with_capacity(dims),
        }
    }

    pub fn dims(&self) -> usize {
        self.dims
    }

    pub fn len(&self) -> usize {
        self.scales.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scales.is_empty()
    }

    /// Bytes held by the matrix (vectors, scales and group tags).
    pub fn heap_bytes(&self) -> usize {
        self.data.capacity() + self.scales.capacity() * 4 + self.groups.capacity() * 4
    }

    /// Adds a vector stored as a little-endian f32 blob. `false` (and nothing
    /// added) when the blob is not exactly `dims` floats, holds a non-finite
    /// value or is the zero vector.
    pub fn push_blob(&mut self, blob: &[u8], group: u32) -> bool {
        if self.dims == 0 || blob.len() != self.dims * 4 {
            return false;
        }
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.clear();
        scratch.extend(
            blob.chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        );
        let added = self.push(&scratch, group);
        self.scratch = scratch;
        added
    }

    /// Adds a vector. Same refusal rules as [`QuantMatrix::push_blob`].
    pub fn push(&mut self, vector: &[f32], group: u32) -> bool {
        if vector.len() != self.dims {
            return false;
        }
        let Some(scale) = quantize_into(vector, &mut self.data) else {
            return false;
        };
        self.scales.push(scale);
        self.groups.push(group);
        true
    }

    /// The `keep` best rows by approximate cosine similarity, best first
    /// (ties to the lower row). `allowed_groups`, when given, is indexed by
    /// group; a row whose group is missing or `false` is skipped. Empty for a
    /// query of another dimension, a zero or a non-finite query.
    pub fn scan(
        &self,
        query: &[f32],
        keep: usize,
        allowed_groups: Option<&[bool]>,
    ) -> Vec<(u32, f32)> {
        if keep == 0 || self.is_empty() || query.len() != self.dims {
            return Vec::new();
        }
        let Some(query) = quantize_query(query) else {
            return Vec::new();
        };
        let order = |a: &(u32, f32), b: &(u32, f32)| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        };
        let mut best: Vec<(u32, f32)> = Vec::with_capacity(keep * 4 + 1);
        let mut floor = f32::NEG_INFINITY;
        for (row, vector) in self.data.chunks_exact(self.dims).enumerate() {
            if let Some(allowed) = allowed_groups {
                if !allowed
                    .get(self.groups[row] as usize)
                    .copied()
                    .unwrap_or(false)
                {
                    continue;
                }
            }
            let score = dot_i8(&query.q, vector) as f32 * query.scale * self.scales[row];
            if score < floor {
                continue;
            }
            best.push((row as u32, score));
            if best.len() >= keep * 4 {
                best.sort_by(order);
                best.truncate(keep);
                floor = best.last().map_or(f32::NEG_INFINITY, |last| last.1);
            }
        }
        best.sort_by(order);
        best.truncate(keep);
        best
    }
}

/// Scales `query` to unit length; `None` for a zero or non-finite query.
pub fn unit(query: &[f32]) -> Option<Vec<f32>> {
    let norm = query
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt();
    if norm == 0.0 || !norm.is_finite() {
        return None;
    }
    Some(
        query
            .iter()
            .map(|value| (f64::from(*value) / norm) as f32)
            .collect(),
    )
}

/// Cosine similarity of `unit_query` (already unit length) against a stored
/// little-endian f32 blob, read in place. Eight independent lanes let the
/// compiler vectorize what a single running sum cannot. `None` when the blob
/// is not exactly `unit_query.len()` floats or is a zero/non-finite vector.
pub fn unit_similarity_blob(unit_query: &[f32], blob: &[u8]) -> Option<f32> {
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

/// Quantizes `vector`, appending its int8 row to `out`, and returns the row
/// scale (`max_abs / 127 / norm`). Nothing is appended on refusal.
fn quantize_into(vector: &[f32], out: &mut Vec<i8>) -> Option<f32> {
    let mut sum = 0.0_f64;
    let mut max_abs = 0.0_f32;
    for value in vector {
        if !value.is_finite() {
            return None;
        }
        sum += f64::from(*value) * f64::from(*value);
        max_abs = max_abs.max(value.abs());
    }
    let norm = sum.sqrt();
    if norm == 0.0 || !norm.is_finite() || max_abs == 0.0 {
        return None;
    }
    let step = max_abs / 127.0;
    out.extend(vector.iter().map(|value| (value / step).round() as i8));
    Some(step / norm as f32)
}

fn quantize_query(query: &[f32]) -> Option<QuantQuery> {
    let mut q = Vec::with_capacity(query.len());
    let scale = quantize_into(query, &mut q)?;
    Some(QuantQuery { q, scale })
}

/// Integer dot product; sixteen lanes of i32 so it vectorizes.
fn dot_i8(a: &[i8], b: &[i8]) -> i32 {
    let mut acc = [0_i32; 16];
    let mut a_lanes = a.chunks_exact(16);
    let mut b_lanes = b.chunks_exact(16);
    for (x, y) in (&mut a_lanes).zip(&mut b_lanes) {
        for lane in 0..16 {
            acc[lane] += i32::from(x[lane]) * i32::from(y[lane]);
        }
    }
    let mut total: i32 = acc.iter().sum();
    for (x, y) in a_lanes.remainder().iter().zip(b_lanes.remainder()) {
        total += i32::from(*x) * i32::from(*y);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random vectors (xorshift), no dependency.
    fn vectors(rows: usize, dims: usize, mut seed: u64) -> Vec<Vec<f32>> {
        (0..rows)
            .map(|_| {
                (0..dims)
                    .map(|_| {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        ((seed % 20_001) as f32 / 10_000.0) - 1.0
                    })
                    .collect()
            })
            .collect()
    }

    fn blob(vector: &[f32]) -> Vec<u8> {
        vector.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn exact_top(rows: &[Vec<f32>], query: &[f32], k: usize) -> Vec<u32> {
        let unit_query = unit(query).unwrap();
        let mut scored: Vec<(u32, f32)> = rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| {
                unit_similarity_blob(&unit_query, &blob(row)).map(|s| (i as u32, s))
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
        scored.into_iter().take(k).map(|(i, _)| i).collect()
    }

    #[test]
    fn the_shortlist_holds_the_exact_top_k() {
        let rows = vectors(2_000, 96, 7);
        let mut matrix = QuantMatrix::new(96);
        for (i, row) in rows.iter().enumerate() {
            assert!(matrix.push_blob(&blob(row), i as u32 % 5));
        }
        for query in vectors(5, 96, 99) {
            let shortlist: Vec<u32> = matrix
                .scan(&query, 40, None)
                .into_iter()
                .map(|(row, _)| row)
                .collect();
            for wanted in exact_top(&rows, &query, 10) {
                assert!(shortlist.contains(&wanted), "row {wanted} missing");
            }
        }
    }

    #[test]
    fn approximate_scores_track_the_exact_cosine() {
        let rows = vectors(300, 128, 3);
        let mut matrix = QuantMatrix::new(128);
        for row in &rows {
            matrix.push(row, 0);
        }
        let query = &vectors(1, 128, 11)[0];
        let unit_query = unit(query).unwrap();
        for (row, score) in matrix.scan(query, 300, None) {
            let exact = unit_similarity_blob(&unit_query, &blob(&rows[row as usize])).unwrap();
            assert!((score - exact).abs() < 0.02, "{score} vs {exact}");
        }
    }

    #[test]
    fn groups_filter_rows_and_unlisted_groups_are_skipped() {
        let rows = vectors(50, 16, 5);
        let mut matrix = QuantMatrix::new(16);
        for (i, row) in rows.iter().enumerate() {
            matrix.push(row, (i % 3) as u32);
        }
        let allowed = [false, true]; // group 2 is unlisted
        let found = matrix.scan(&rows[1], 50, Some(&allowed));
        assert!(!found.is_empty());
        assert!(found.iter().all(|(row, _)| row % 3 == 1));
        assert_eq!(found[0].0, 1, "a row is its own nearest neighbour");
    }

    #[test]
    fn unusable_vectors_and_queries_are_refused() {
        let mut matrix = QuantMatrix::new(4);
        assert!(!matrix.push(&[0.0; 4], 0), "zero vector");
        assert!(!matrix.push(&[1.0, f32::NAN, 0.0, 0.0], 0), "non-finite");
        assert!(!matrix.push(&[1.0; 3], 0), "wrong dimension");
        assert!(!matrix.push_blob(&[0; 15], 0), "torn blob");
        assert!(matrix.is_empty());
        assert!(matrix.push(&[1.0, 0.0, 0.0, 0.0], 0));
        assert!(matrix.scan(&[0.0; 4], 5, None).is_empty());
        assert!(matrix.scan(&[1.0; 3], 5, None).is_empty());
        assert_eq!(matrix.scan(&[2.0, 0.0, 0.0, 0.0], 5, None).len(), 1);
    }

    #[test]
    fn exact_similarity_matches_a_plain_cosine() {
        let a = [0.3_f32, -1.25, 2.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.5];
        let b = [1.5_f32, 0.25, -0.75, 4.0, 1.0, 0.0, 2.0, 1.0, 0.5];
        let dot: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        let got = unit_similarity_blob(&unit(&a).unwrap(), &blob(&b)).unwrap();
        assert!((got - dot / (na * nb)).abs() < 1e-5);
        assert_eq!(
            unit_similarity_blob(&unit(&a).unwrap(), &blob(&b)[..8]),
            None
        );
    }
}
