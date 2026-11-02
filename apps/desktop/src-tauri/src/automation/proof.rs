//! The instance proof that keeps a stale discovery file harmless (D12).
//!
//! After a crash the app cannot remove `agent-endpoint.json`, so for a moment it
//! names a port another process — possibly another OS user — may hold. That
//! process would otherwise receive a valid bearer token on the connector's first
//! request. The proof closes that window without putting a secret on the wire:
//! each start mints a 32-byte `instanceKey`, writes it into the discovery file
//! (which is `0600`) and never sends it over HTTP. A caller proves it read the
//! file by echoing a challenge it chose, and the endpoint answers with
//! `HMAC-SHA256(instanceKey, challenge)`; the connector compares that in constant
//! time before it sends any credential.
//!
//! HMAC is built on the `sha2` dependency the endpoint already uses, and the RFC
//! 4231 vector is a test here (and in the connector), so both sides implement the
//! same function.

use sha2::{Digest, Sha256};

/// The request header that carries a caller's challenge (32 bytes hex).
pub const CHALLENGE_HEADER: &str = "x-entropia-challenge";

/// The response header that carries the endpoint's proof.
pub const PROOF_HEADER: &str = "x-entropia-proof";

/// Random bytes behind one instance key, and behind one challenge.
pub const INSTANCE_KEY_BYTES: usize = 32;

/// Length of an instance key or a challenge once hex-encoded.
pub const HEX_CHARS: usize = INSTANCE_KEY_BYTES * 2;

/// HMAC-SHA256 block size, in bytes.
const HMAC_BLOCK_BYTES: usize = 64;

/// This start's instance key: [`INSTANCE_KEY_BYTES`] bytes from the OS
/// generator, hex-encoded.
pub fn mint_instance_key() -> Result<String, String> {
    let mut bytes = [0u8; INSTANCE_KEY_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("could not read the OS random generator: {error}"))?;
    Ok(hex::encode(bytes))
}

/// Whether `value` is a well-formed challenge or instance key: exactly
/// [`HEX_CHARS`] hexadecimal characters.
pub fn is_hex_64(value: &str) -> bool {
    value.len() == HEX_CHARS && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The proof for one challenge: `hex(HMAC-SHA256(instanceKey, challenge))`,
/// with the key and the challenge taken as their ASCII text, exactly as the
/// connector reads them from the discovery file and sends them on the wire.
pub fn proof_for(instance_key: &str, challenge: &str) -> String {
    hex::encode(hmac_sha256(instance_key.as_bytes(), challenge.as_bytes()))
}

/// Compare two byte strings without an early exit, so a wrong proof cannot be
/// found byte by byte.
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

/// HMAC-SHA256 (RFC 2104) over `sha2`, which is already in the tree.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0u8; HMAC_BLOCK_BYTES];
    if key.len() > HMAC_BLOCK_BYTES {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }

    let mut inner = Sha256::new();
    inner.update(block.map(|byte| byte ^ 0x36));
    inner.update(message);
    let inner = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(block.map(|byte| byte ^ 0x5c));
    outer.update(inner);
    let outer = outer.finalize();

    let mut mac = [0u8; 32];
    mac.copy_from_slice(&outer);
    mac
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4231, test case 2: the vector the connector is pinned with too, so the
    /// two implementations cannot drift apart.
    #[test]
    fn hmac_sha256_matches_rfc_4231_test_case_2() {
        assert_eq!(
            hex::encode(hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn a_key_and_a_challenge_are_32_bytes_of_hex() {
        let key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(is_hex_64(key));
        assert!(!is_hex_64(&key[..63]));
        assert!(!is_hex_64(&format!("{}g", &key[..63])));
        assert!(!is_hex_64(""));
        let minted = mint_instance_key().expect("the OS generator answers");
        assert!(is_hex_64(&minted), "{minted}");
        assert_ne!(minted, mint_instance_key().expect("mint again"));
    }

    #[test]
    fn a_comparison_is_only_true_for_equal_bytes() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"a"));
    }
}
