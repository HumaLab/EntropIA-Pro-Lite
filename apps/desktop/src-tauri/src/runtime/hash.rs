//! SHA-256 of runtime files without loading them into memory.
//!
//! Runtime pack entries include the Python distribution and native libraries,
//! some of them hundreds of megabytes; reading each one whole just to hash it
//! spiked memory during install and every integrity check.

use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// Read buffer size: large enough that hashing is not syscall-bound.
const BUFFER_BYTES: usize = 1024 * 1024;

/// Lowercase hex SHA-256 of the file at `path`, read in 1 MiB chunks.
pub(crate) fn sha256_file_streaming(path: &Path) -> std::io::Result<String> {
    let mut reader = BufReader::with_capacity(BUFFER_BYTES, File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_of(bytes: &[u8]) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("entry.bin");
        std::fs::write(&path, bytes).unwrap();
        sha256_file_streaming(&path).unwrap()
    }

    #[test]
    fn matches_the_one_shot_digest_for_empty_small_and_multi_buffer_files() {
        let multi_buffer: Vec<u8> = (0..(BUFFER_BYTES * 2 + 12_345))
            .map(|index| (index % 251) as u8)
            .collect();
        for bytes in [&b""[..], &b"python"[..], &multi_buffer[..]] {
            assert_eq!(hash_of(bytes), format!("{:x}", Sha256::digest(bytes)));
        }
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        assert!(sha256_file_streaming(&dir.path().join("missing")).is_err());
    }
}
