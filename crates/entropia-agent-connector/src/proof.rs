//! The instance proof (D12): what the connector checks before it sends a token.
//!
//! A stale `agent-endpoint.json` after a crash names a port this app no longer
//! holds; whatever process holds it next would receive a valid bearer token on
//! the first request. So each app start mints a 32-byte `instanceKey`, writes it
//! into the `0600` discovery file and never sends it over HTTP. The connector
//! sends a challenge it chose and compares the `X-Entropia-Proof` answer —
//! `HMAC-SHA256(instanceKey, challenge)` over the ASCII text of both — in
//! constant time. Only a match allows the token to leave.
//!
//! HMAC is built on `sha2` (the same implementation the app has, pinned by the
//! same RFC 4231 vector) and the hex encoding is local, so verifying an instance
//! costs one small dependency rather than a crypto stack.

use sha2::{Digest, Sha256};

/// The request header that carries the connector's challenge.
pub const CHALLENGE_HEADER: &str = "x-entropia-challenge";

/// The response header the app answers a challenge with.
pub const PROOF_HEADER: &str = "x-entropia-proof";

/// Random bytes in a challenge and in an instance key.
pub const KEY_BYTES: usize = 32;

/// Length of a challenge or an instance key once hex-encoded.
pub const HEX_CHARS: usize = KEY_BYTES * 2;

/// HMAC-SHA256 block size, in bytes.
const HMAC_BLOCK_BYTES: usize = 64;

/// A fresh challenge: [`KEY_BYTES`] bytes from the OS generator, hex-encoded.
pub fn challenge() -> Result<String, String> {
    let mut bytes = [0u8; KEY_BYTES];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| format!("could not read the OS random generator: {error}"))?;
    Ok(hex_encode(&bytes))
}

/// Whether `value` is exactly [`HEX_CHARS`] hexadecimal characters, which is
/// what an instance key and a challenge are.
pub fn is_hex_64(value: &str) -> bool {
    value.len() == HEX_CHARS && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The proof for one challenge, from the key text in the discovery file:
/// `hex(HMAC-SHA256(instanceKey, challenge))`.
pub fn proof_for(instance_key: &str, challenge: &str) -> String {
    hex_encode(&hmac_sha256(instance_key.as_bytes(), challenge.as_bytes()))
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

/// HMAC-SHA256 (RFC 2104) over `sha2`.
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

/// Lower-case hex, so the connector needs no extra crate for it.
fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4231, test case 2. The app has the same vector, so the two
    /// implementations cannot drift apart.
    #[test]
    fn hmac_sha256_matches_rfc_4231_test_case_2() {
        assert_eq!(
            hex_encode(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn a_challenge_is_32_random_bytes_of_hex() {
        let first = challenge().expect("the OS generator answers");
        assert!(is_hex_64(&first), "{first}");
        assert_ne!(first, challenge().expect("challenge again"));
        assert!(!is_hex_64(&first[..63]));
        assert!(!is_hex_64(&format!("{}g", &first[..63])));
    }

    #[test]
    fn only_equal_bytes_compare_equal() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"a"));
    }
}
