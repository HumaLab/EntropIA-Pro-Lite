//! Resolve user input (bare identifier or archive.org URL) to an identifier.
//!
//! Items and collections share the same URL shape
//! `https://archive.org/details/<identifier>`; only the item's
//! `metadata.mediatype` tells them apart, so resolution stops at the
//! identifier and never performs I/O. Callers pass the result to
//! `get_item`/`search`, never fetching a URL built from user input.

use url::Url;

use crate::error::IaError;
use crate::search::is_valid_identifier;

/// Resolve `input` to an archive.org identifier.
///
/// A bare identifier valid under [`is_valid_identifier`] is returned as is.
/// Otherwise the input must be an `http(s)` URL on exactly `archive.org` or
/// `www.archive.org` with a `/details/<id>` or `/download/<id>` path; the
/// `<id>` segment is percent-decoded once and must itself be a valid
/// identifier. Everything else is [`IaError::InvalidQuery`].
pub fn resolve_identifier(input: &str) -> Result<String, IaError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(IaError::InvalidQuery {
            reason: "empty input; expected a bare identifier or an archive.org URL".to_string(),
        });
    }
    if is_valid_identifier(trimmed) {
        return Ok(trimmed.to_string());
    }
    let url = Url::parse(trimmed).map_err(|_| IaError::InvalidQuery {
        reason: format!("not an identifier or archive.org URL: {trimmed:?}"),
    })?;
    match url.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(IaError::InvalidQuery {
                reason: format!("unsupported URL scheme {scheme:?}; expected http or https"),
            });
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(IaError::InvalidQuery {
            reason: "archive.org URLs must not carry userinfo".to_string(),
        });
    }
    // `Url` normalizes an explicit scheme-default port away, so any
    // surviving port is a non-default one.
    if url.port().is_some() {
        return Err(IaError::InvalidQuery {
            reason: "archive.org URLs must not carry an explicit non-default port".to_string(),
        });
    }
    match url.host_str() {
        // `host_str` is already lowercased for domain names.
        Some("archive.org") | Some("www.archive.org") => {}
        Some(host) => {
            return Err(IaError::InvalidQuery {
                reason: format!(
                    "unexpected host {host:?}; expected archive.org or www.archive.org"
                ),
            });
        }
        None => {
            return Err(IaError::InvalidQuery {
                reason: "archive.org URL has no host".to_string(),
            });
        }
    }
    // `path_segments` splits on raw `/` without percent-decoding, so an
    // encoded slash (`%2F`) stays inside its segment and is rejected below
    // when the decoded value fails identifier validation. Query strings
    // and fragments never reach the path segments and are ignored.
    let segments: Vec<&str> = url
        .path_segments()
        .map(Iterator::collect)
        .unwrap_or_default();
    let raw = match segments.as_slice() {
        ["details", id, ..] | ["download", id, ..] if !id.is_empty() => *id,
        _ => {
            return Err(IaError::InvalidQuery {
                reason: format!(
                    "unsupported archive.org path {:?}; expected /details/<id> or /download/<id>",
                    url.path()
                ),
            });
        }
    };
    let id = percent_decode_once(raw).ok_or_else(|| IaError::InvalidQuery {
        reason: format!("identifier in URL path is not decodable: {raw:?}"),
    })?;
    if is_valid_identifier(&id) {
        Ok(id)
    } else {
        Err(IaError::InvalidQuery {
            reason: format!("invalid identifier in URL path: {raw:?}"),
        })
    }
}

/// Percent-decode `raw` exactly once (`%XX` with hex digits, `+` kept
/// literal as befits a path segment). Returns `None` when a `%` escape is
/// malformed or the result is not UTF-8.
fn percent_decode_once(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).and_then(|h| {
                std::str::from_utf8(h)
                    .ok()
                    .and_then(|s| u8::from_str_radix(s, 16).ok())
            });
            match hex {
                Some(byte) => {
                    out.push(byte);
                    i += 3;
                }
                None => return None,
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "my-item_01";

    #[test]
    fn accept_table() {
        let cases = [
            ID,
            "https://archive.org/details/my-item_01",
            "http://archive.org/details/my-item_01",
            "https://www.archive.org/details/my-item_01/",
            "https://archive.org/details/my-item_01?tab=collection",
            "https://archive.org/details/my-item_01/page/n5/mode/2up",
            "https://archive.org/download/my-item_01/file.pdf",
            "https://ARCHIVE.org/details/my-item_01",
            "  my-item_01  ",
            "  https://archive.org/details/my-item_01  ",
            "https://archive.org/details/my%2Ditem%5F01",
            "https://archive.org/details/my%2ditem%5f01",
        ];
        for input in cases {
            match resolve_identifier(input) {
                Ok(id) => assert_eq!(id, ID, "input {input:?} should resolve"),
                Err(e) => panic!("input {input:?} should resolve, got {e:?}"),
            }
        }
    }

    #[test]
    fn resolved_value_is_never_a_url() {
        for input in [
            "https://archive.org/details/my-item_01",
            "https://archive.org/download/my-item_01/file.pdf",
        ] {
            let id = resolve_identifier(input).unwrap();
            assert!(!id.contains("://"), "must be an identifier, got {id:?}");
            assert!(!id.contains('/'), "must be an identifier, got {id:?}");
        }
    }

    #[test]
    fn reject_table() {
        let cases = [
            "",
            "   ",
            "has space",
            "a/b",
            "../evil",
            "https://archive.org.evil.com/details/x",
            "https://web.archive.org/details/x",
            "https://example.com/details/x",
            "https://archive.org/search?query=x",
            "https://archive.org/",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "ftp://archive.org/details/x",
            "https://archive.org:8443/details/x",
            "https://user@archive.org/details/x",
            "https://user:pass@archive.org/details/x",
            "https://archive.org/details/",
            "https://archive.org/details",
            "https://archive.org/details/a%2Fb",
            "https://archive.org/details/a%2fb",
            "https://archive.org/details/%",
            "https://archive.org/download/",
        ];
        for input in cases {
            let got = resolve_identifier(input);
            assert!(
                matches!(got, Err(IaError::InvalidQuery { .. })),
                "input {input:?} should be InvalidQuery, got {got:?}"
            );
        }
    }
}
