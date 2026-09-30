//! Shared helpers for embedding vectors stored as little-endian f32 blobs.
//!
//! Used by the asset-similarity commands (`nlp::commands`) and the RAG
//! retrieval pipeline.

/// Decode an embedding blob (f32 little-endian) into a vector.
pub(crate) fn decode_embedding_blob(blob: &[u8]) -> Result<Vec<f32>, String> {
    if blob.len() % 4 != 0 {
        return Err(format!(
            "Embedding blob has invalid size: {} bytes (not divisible by 4)",
            blob.len()
        ));
    }

    Ok(blob
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
        .collect())
}

/// [`cosine_distance`] against a stored little-endian f32 blob, read in
/// place. Same arithmetic as decoding first, without allocating a vector per
/// row: a vector search streams every stored embedding through this. `None`
/// when the blob is not exactly `a.len()` floats, as decode + compare gives.
pub(crate) fn cosine_distance_to_blob(a: &[f32], blob: &[u8]) -> Option<f64> {
    if a.is_empty() || blob.len() != a.len() * 4 {
        return None;
    }

    let mut dot = 0.0_f64;
    let mut mag_a = 0.0_f64;
    let mut mag_b = 0.0_f64;

    for (ai, bytes) in a.iter().zip(blob.chunks_exact(4)) {
        let ai = *ai as f64;
        let bi = f32::from_le_bytes(bytes.try_into().unwrap()) as f64;
        dot += ai * bi;
        mag_a += ai * ai;
        mag_b += bi * bi;
    }

    let mag_a = mag_a.sqrt();
    let mag_b = mag_b.sqrt();

    if mag_a == 0.0 || mag_b == 0.0 {
        return None;
    }

    Some(1.0 - dot / (mag_a * mag_b))
}

/// Compute cosine distance (1 - cosine_similarity) between two f32 vectors.
/// Returns None if either vector has zero magnitude.
pub(crate) fn cosine_distance(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }

    let mut dot = 0.0_f64;
    let mut mag_a = 0.0_f64;
    let mut mag_b = 0.0_f64;

    for (ai, bi) in a.iter().zip(b.iter()) {
        let ai = *ai as f64;
        let bi = *bi as f64;
        dot += ai * bi;
        mag_a += ai * ai;
        mag_b += bi * bi;
    }

    let mag_a = mag_a.sqrt();
    let mag_b = mag_b.sqrt();

    if mag_a == 0.0 || mag_b == 0.0 {
        return None;
    }

    Some(1.0 - dot / (mag_a * mag_b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_the_blob_in_place_matches_decoding_it_first() {
        let query = [0.3_f32, -1.25, 2.0, 0.0];
        let stored = [1.5_f32, 0.25, -0.75, 4.0];
        let blob: Vec<u8> = stored.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(
            cosine_distance_to_blob(&query, &blob),
            cosine_distance(&query, &decode_embedding_blob(&blob).unwrap())
        );
        // Another dimension, a torn blob or a zero vector compare as nothing.
        assert_eq!(cosine_distance_to_blob(&query[..3], &blob), None);
        assert_eq!(cosine_distance_to_blob(&query, &blob[..15]), None);
        assert_eq!(cosine_distance_to_blob(&query, &[0; 16]), None);
    }
}
