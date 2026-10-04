//! Compact checkpoint encoding for embedding vectors.
//!
//! A checkpoint payload is JSON text. A 1024-dimension vector written as a
//! JSON array of floats takes ~11 KB; the same vector as base64 of its
//! little-endian f32 bytes takes ~5.5 KB, about half. Reading stays
//! backward compatible: a checkpoint written before this encoding holds a
//! plain JSON array and still deserializes, so units already in flight when
//! the build changes resume instead of recomputing.

use base64::prelude::{Engine as _, BASE64_STANDARD};
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// One embedding vector, stored as base64 of its little-endian f32 bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactVec(pub Vec<f32>);

impl Serialize for CompactVec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = self
            .0
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        serializer.serialize_str(&BASE64_STANDARD.encode(bytes))
    }
}

impl<'de> Deserialize<'de> for CompactVec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct VecVisitor;
        impl<'de> Visitor<'de> for VecVisitor {
            type Value = CompactVec;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a base64 f32 vector or a JSON array of floats")
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<CompactVec, E> {
                let bytes = BASE64_STANDARD
                    .decode(text)
                    .map_err(|error| E::custom(format!("corrupt compact vector: {error}")))?;
                if bytes.len() % 4 != 0 {
                    return Err(E::custom("truncated compact vector"));
                }
                Ok(CompactVec(
                    bytes
                        .chunks_exact(4)
                        .map(|word| f32::from_le_bytes([word[0], word[1], word[2], word[3]]))
                        .collect(),
                ))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<CompactVec, A::Error> {
                let mut values = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                while let Some(value) = seq.next_element::<f32>()? {
                    values.push(value);
                }
                Ok(CompactVec(values))
            }
        }
        deserializer.deserialize_any(VecVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<f32> {
        (0..1024).map(|i| (i as f32).sin() * 0.37 - 0.011).collect()
    }

    #[test]
    fn a_compact_vector_round_trips_bit_exactly() {
        let original = CompactVec(sample());
        let json = serde_json::to_string(&original).unwrap();
        let back: CompactVec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn a_wave_of_vectors_round_trips() {
        let wave = vec![CompactVec(sample()), CompactVec(vec![1.5, -2.0, 0.0])];
        let json = serde_json::to_string(&wave).unwrap();
        let back: Vec<CompactVec> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, wave);
    }

    #[test]
    fn a_pair_with_a_label_round_trips() {
        let value = (CompactVec(vec![0.25, 0.5]), "local".to_string());
        let json = serde_json::to_string(&value).unwrap();
        let back: (CompactVec, String) = serde_json::from_str(&json).unwrap();
        assert_eq!(back, value);
    }

    #[test]
    fn checkpoints_written_as_plain_json_arrays_still_read() {
        let vector = sample();
        let legacy = serde_json::to_string(&vector).unwrap();
        let back: CompactVec = serde_json::from_str(&legacy).unwrap();
        assert_eq!(back.0, vector);
        let legacy_wave = serde_json::to_string(&vec![vector.clone(), vec![3.0]]).unwrap();
        let back: Vec<CompactVec> = serde_json::from_str(&legacy_wave).unwrap();
        assert_eq!(back[0].0, vector);
        assert_eq!(back[1].0, vec![3.0]);
        let legacy_pair = serde_json::to_string(&(vec![1.0_f32, 2.0], "local")).unwrap();
        let back: (CompactVec, String) = serde_json::from_str(&legacy_pair).unwrap();
        assert_eq!(back.0 .0, vec![1.0, 2.0]);
    }

    #[test]
    fn the_compact_form_is_about_half_the_json_array() {
        let vector = sample();
        let legacy = serde_json::to_string(&vector).unwrap().len();
        let compact = serde_json::to_string(&CompactVec(vector)).unwrap().len();
        assert!(
            compact * 2 <= legacy + legacy / 10,
            "compact {compact} vs legacy {legacy}"
        );
    }

    #[test]
    fn a_corrupt_or_truncated_payload_is_an_error_not_a_vector() {
        assert!(serde_json::from_str::<CompactVec>("\"!!not base64!!\"").is_err());
        // 3 bytes is not a whole f32.
        assert!(serde_json::from_str::<CompactVec>("\"AAEC\"").is_err());
    }
}
