//! Item metadata: tolerant parsing of the Metadata API response.
//!
//! Every metadata field may arrive as a string, an array, a number, a bool
//! or be missing entirely; unparseable `size`/`mtime` values become `None`
//! instead of failing the item. PDF detection looks at `format` first and
//! then at the file name, never assuming a download ends in `.pdf`.

use serde_json::Value;

use crate::error::IaError;
use crate::search::{first_string, flag_in, scalar_string, string_list};

/// How many PDF-ish entries are kept per item, in response order. The whole
/// `files[]` array is still walked; when more PDF-ish entries existed the
/// item reports [`ItemDetails::truncated`].
pub const MAX_PDFS_PER_ITEM: usize = 1000;

/// Normalized item metadata with its PDF file list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemDetails {
    /// The requested identifier.
    pub identifier: String,
    /// First `title` string, if any (source text preserved).
    pub title: Option<String>,
    /// Normalized `creator` values.
    pub creators: Vec<String>,
    /// First `date` string, if any (source text preserved).
    pub date: Option<String>,
    /// Normalized `language` values.
    pub languages: Vec<String>,
    /// Normalized `collection` values.
    pub collections: Vec<String>,
    /// First `rights` string, if any.
    pub rights: Option<String>,
    /// First `licenseurl` string, if any.
    pub license_url: Option<String>,
    /// First `mediatype` string, if any (`collection` for collections:
    /// items and collections share the same `/details/<id>` URL shape).
    pub mediatype: Option<String>,
    /// True when `metadata.access-restricted-item` is truthy.
    pub restricted: bool,
    /// PDF-ish files in response order, capped at [`MAX_PDFS_PER_ITEM`].
    pub pdfs: Vec<PdfFile>,
    /// True when more PDF-ish entries existed than were kept.
    pub truncated: bool,
}

impl ItemDetails {
    /// True when the item is a collection: `mediatype` equals `collection`
    /// (case-insensitive). Missing or any other mediatype is not a
    /// collection.
    pub fn is_collection(&self) -> bool {
        self.mediatype
            .as_deref()
            .is_some_and(|m| m.trim().eq_ignore_ascii_case("collection"))
    }
}

/// One PDF-ish file entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfFile {
    /// File name exactly as published by the Metadata API.
    pub name: String,
    /// Declared format, if any.
    pub format: Option<String>,
    /// Declared `source` bucket.
    pub source: FileSource,
    /// File size in bytes: number or decimal string, else `None`.
    pub size: Option<u64>,
    /// Modification time (unix seconds): number or decimal string, else `None`.
    pub mtime: Option<u64>,
    /// Declared MD5, if any.
    pub md5: Option<String>,
    /// Declared SHA-1, if any.
    pub sha1: Option<String>,
    /// Availability decided by the first matching rule.
    pub availability: Availability,
}

/// Declared `source` bucket of a file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileSource {
    /// `source` is `original` (any case).
    Original,
    /// `source` is `derivative` (any case).
    Derivative,
    /// Any other declared value, preserved verbatim.
    Other(String),
    /// Missing or unusable `source`.
    Unknown,
}

/// Availability of a PDF-ish file. `Available` means no restriction signal
/// was seen ("presumably available"); the download response stays the final
/// authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// No restriction signal.
    Available,
    /// A restriction signal matched; see the reason.
    Restricted(RestrictedReason),
}

/// Why a file is not offered for download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestrictedReason {
    /// `format` contains `encrypted` or the name ends in `.lcpdf`.
    Encrypted,
    /// The file's `private` flag is truthy.
    Private,
    /// The item itself is access-restricted.
    ItemRestricted,
    /// The published name is not a safe relative path.
    UnsafeName,
}

/// Build an [`ItemDetails`] from a raw Metadata API value. Only a missing
/// (or non-object) `metadata` fails, with [`IaError::NotFound`]: the API
/// answers `{}` for unknown identifiers. Everything else is tolerant.
pub fn parse_item_value(identifier: &str, body: &Value) -> Result<ItemDetails, IaError> {
    let not_found = || IaError::NotFound {
        identifier: identifier.to_string(),
    };
    let metadata = body
        .get("metadata")
        .and_then(Value::as_object)
        .ok_or_else(not_found)?;
    let restricted = flag_in(metadata, "access-restricted-item");
    let mut pdf_seen = 0usize;
    let mut pdfs = Vec::new();
    if let Some(files) = body.get("files").and_then(Value::as_array) {
        for file in files {
            let Some(entry) = file.as_object() else {
                continue;
            };
            let name = file_name_in(entry);
            let format = entry.get("format").and_then(scalar_string);
            if !is_pdf_ish(&name, format.as_deref()) {
                continue;
            }
            pdf_seen += 1;
            if pdfs.len() >= MAX_PDFS_PER_ITEM {
                continue;
            }
            pdfs.push(PdfFile {
                availability: availability_in(entry, &name, format.as_deref(), restricted),
                name,
                format,
                source: file_source_in(entry),
                size: entry.get("size").and_then(decimal_u64),
                mtime: entry.get("mtime").and_then(decimal_u64),
                md5: entry.get("md5").and_then(scalar_string),
                sha1: entry.get("sha1").and_then(scalar_string),
            });
        }
    }
    Ok(ItemDetails {
        identifier: identifier.to_string(),
        title: metadata.get("title").and_then(first_string),
        creators: string_list(metadata.get("creator").or_else(|| metadata.get("creators"))),
        date: metadata.get("date").and_then(first_string),
        languages: string_list(
            metadata
                .get("language")
                .or_else(|| metadata.get("languages")),
        ),
        collections: string_list(
            metadata
                .get("collection")
                .or_else(|| metadata.get("collections")),
        ),
        rights: metadata.get("rights").and_then(first_string),
        license_url: license_url_in(metadata),
        mediatype: metadata.get("mediatype").and_then(first_string),
        restricted,
        truncated: pdf_seen > pdfs.len(),
        pdfs,
    })
}

/// `licenseurl` as published (exact key first, any-case fallback).
fn license_url_in(metadata: &serde_json::Map<String, Value>) -> Option<String> {
    metadata
        .get("licenseurl")
        .and_then(first_string)
        .or_else(|| {
            metadata
                .iter()
                .filter(|(key, _)| {
                    key.as_str() != "licenseurl" && key.eq_ignore_ascii_case("licenseurl")
                })
                .filter_map(|(_, value)| first_string(value))
                .next()
        })
}

/// File name exactly as published; non-string scalars stringified, anything
/// else (missing, null, array, object) becomes `""` so the entry still
/// classifies as [`RestrictedReason::UnsafeName`] when PDF-ish by format.
fn file_name_in(file: &serde_json::Map<String, Value>) -> String {
    match file.get("name") {
        Some(Value::String(name)) => name.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// Declared `source` bucket: `original`/`derivative` in any case, any other
/// value preserved verbatim, missing or empty is [`FileSource::Unknown`].
fn file_source_in(file: &serde_json::Map<String, Value>) -> FileSource {
    match file.get("source").and_then(first_string) {
        None => FileSource::Unknown,
        Some(raw) if raw.eq_ignore_ascii_case("original") => FileSource::Original,
        Some(raw) if raw.eq_ignore_ascii_case("derivative") => FileSource::Derivative,
        Some(raw) => FileSource::Other(raw),
    }
}

/// A file is PDF-ish when `format` mentions `pdf` (any case) or the name
/// ends in `.pdf` or `.lcpdf` (any case).
fn is_pdf_ish(name: &str, format: Option<&str>) -> bool {
    if format.is_some_and(|f| f.to_ascii_lowercase().contains("pdf")) {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".pdf") || lower.ends_with(".lcpdf")
}

/// Availability by first matching rule: `Encrypted`, then `Private`, then
/// `ItemRestricted`, then `UnsafeName`, otherwise `Available`.
fn availability_in(
    file: &serde_json::Map<String, Value>,
    name: &str,
    format: Option<&str>,
    item_restricted: bool,
) -> Availability {
    if format.is_some_and(|f| f.to_ascii_lowercase().contains("encrypted"))
        || name.to_ascii_lowercase().ends_with(".lcpdf")
    {
        return Availability::Restricted(RestrictedReason::Encrypted);
    }
    if flag_in(file, "private") {
        return Availability::Restricted(RestrictedReason::Private);
    }
    if item_restricted {
        return Availability::Restricted(RestrictedReason::ItemRestricted);
    }
    if is_unsafe_name(name) {
        return Availability::Restricted(RestrictedReason::UnsafeName);
    }
    Availability::Available
}

/// True when the published name is not a safe relative path: empty,
/// absolute, carrying a `..` segment or a backslash, or holding control
/// characters.
fn is_unsafe_name(name: &str) -> bool {
    name.is_empty()
        || name.starts_with('/')
        || name.contains('\\')
        || name.split('/').any(|segment| segment == "..")
        || name.chars().any(|c| c.is_control())
}

/// `size`/`mtime` arrive as a number or a decimal string; missing, null or
/// unparseable (`"12a"`) is `None` rather than an item failure.
fn decimal_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|i| u64::try_from(i).ok()))
            .or_else(|| {
                n.as_f64().and_then(|f| {
                    if f >= 0.0 && f <= u64::MAX as f64 && f.fract() == 0.0 {
                        Some(f as u64)
                    } else {
                        None
                    }
                })
            }),
        Value::String(raw) => raw.trim().parse::<u64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::FetchPolicy;
    use crate::test_support::TestServer;
    use crate::{Endpoints, IaClient, IaError};
    use serde_json::json;
    use url::Url;

    const OPEN_FIXTURE: &str = include_str!("../tests/fixtures/metadata-open-text-pdf.json");
    const LENDING_FIXTURE: &str = include_str!("../tests/fixtures/metadata-lending-encrypted.json");
    const OPEN_ID: &str = "30-festival-notas-para-una-contra-historia-del-cine-argentino";
    const LENDING_ID: &str = "proceedingsofvic00cong";

    fn local_endpoints(server: &TestServer) -> Endpoints {
        Endpoints::new(Url::parse(server.base()).unwrap())
    }

    fn local_client() -> IaClient {
        IaClient::new(FetchPolicy::loopback_for_tests())
    }

    fn metadata_route(path: &str, body: &str) -> crate::test_support::Route {
        TestServer::json(path, 200, body)
    }

    #[tokio::test]
    async fn open_fixture_single_available_pdf() {
        let server = TestServer::start(vec![metadata_route(
            &format!("/metadata/{OPEN_ID}"),
            OPEN_FIXTURE,
        )]);
        let details = local_client()
            .get_item(&local_endpoints(&server), OPEN_ID)
            .await
            .unwrap();
        assert_eq!(details.identifier, OPEN_ID);
        assert_eq!(
            details.title.as_deref(),
            Some(
                "30º Festival Internacional de Cine de Mar del Plata (2015) - \
                 Notas para una contrahistoria del cine argentino"
            )
        );
        assert!(details.creators.is_empty());
        assert!(details.date.is_none());
        assert!(details.languages.is_empty());
        assert_eq!(details.collections, vec!["opensource"]);
        assert!(details.rights.is_none());
        assert!(details.license_url.is_none());
        assert!(!details.restricted);
        assert!(!details.truncated);
        assert_eq!(details.pdfs.len(), 1);
        let pdf = &details.pdfs[0];
        assert_eq!(
            pdf.name,
            "30-Festival-Notas-para-una-contra-historia-del-cine-argentino.pdf"
        );
        assert_eq!(pdf.format.as_deref(), Some("Text PDF"));
        assert_eq!(pdf.source, FileSource::Original);
        assert_eq!(pdf.size, Some(1_489_787));
        assert_eq!(pdf.mtime, Some(1_788_289_643));
        assert_eq!(pdf.md5.as_deref(), Some("884bfe316f489fd421089ede8afab665"));
        assert_eq!(
            pdf.sha1.as_deref(),
            Some("8eb49467a31dbc35088346b90c0d381e63fcce56")
        );
        assert_eq!(pdf.availability, Availability::Available);
    }

    #[tokio::test]
    async fn lending_fixture_three_pdfs_all_non_available() {
        let server = TestServer::start(vec![metadata_route(
            &format!("/metadata/{LENDING_ID}"),
            LENDING_FIXTURE,
        )]);
        let details = local_client()
            .get_item(&local_endpoints(&server), LENDING_ID)
            .await
            .unwrap();
        assert!(details.restricted);
        assert!(!details.truncated);
        assert_eq!(details.creators.len(), 3);
        assert_eq!(details.date.as_deref(), Some("1998"));
        assert_eq!(details.languages, vec!["eng", "spa", "por"]);
        assert_eq!(details.pdfs.len(), 3);
        assert_eq!(details.pdfs[0].name, "proceedingsofvic00cong.lcpdf");
        assert_eq!(
            details.pdfs[0].availability,
            Availability::Restricted(RestrictedReason::Encrypted)
        );
        assert_eq!(details.pdfs[1].name, "proceedingsofvic00cong.pdf");
        assert_eq!(
            details.pdfs[1].availability,
            Availability::Restricted(RestrictedReason::Private)
        );
        assert_eq!(details.pdfs[2].name, "proceedingsofvic00cong_encrypted.pdf");
        assert_eq!(
            details.pdfs[2].availability,
            Availability::Restricted(RestrictedReason::Encrypted)
        );
    }

    #[tokio::test]
    async fn available_pdf_in_restricted_item_is_item_restricted() {
        let body = json!({
            "metadata": {"identifier": "r", "access-restricted-item": "true"},
            "files": [{"name": "doc.pdf", "format": "Text PDF", "source": "original"}],
        })
        .to_string();
        let server = TestServer::start(vec![metadata_route("/metadata/r", &body)]);
        let details = local_client()
            .get_item(&local_endpoints(&server), "r")
            .await
            .unwrap();
        assert!(details.restricted);
        assert_eq!(details.pdfs.len(), 1);
        assert_eq!(
            details.pdfs[0].availability,
            Availability::Restricted(RestrictedReason::ItemRestricted)
        );
    }

    #[tokio::test]
    async fn size_and_mtime_accept_number_string_missing_and_garbage() {
        let body = json!({
            "metadata": {"identifier": "s"},
            "files": [
                {"name": "a.pdf", "format": "Text PDF", "size": 123, "mtime": 456},
                {"name": "b.pdf", "format": "Text PDF", "size": "789", "mtime": "101112"},
                {"name": "c.pdf", "format": "Text PDF"},
                {"name": "d.pdf", "format": "Text PDF", "size": "12a", "mtime": "xx"},
            ],
        })
        .to_string();
        let server = TestServer::start(vec![metadata_route("/metadata/s", &body)]);
        let details = local_client()
            .get_item(&local_endpoints(&server), "s")
            .await
            .unwrap();
        assert_eq!(details.pdfs.len(), 4);
        assert_eq!(details.pdfs[0].size, Some(123));
        assert_eq!(details.pdfs[0].mtime, Some(456));
        assert_eq!(details.pdfs[1].size, Some(789));
        assert_eq!(details.pdfs[1].mtime, Some(101_112));
        assert_eq!(details.pdfs[2].size, None);
        assert_eq!(details.pdfs[2].mtime, None);
        assert_eq!(details.pdfs[3].size, None);
        assert_eq!(details.pdfs[3].mtime, None);
        for pdf in &details.pdfs {
            assert_eq!(pdf.availability, Availability::Available);
        }
    }

    #[tokio::test]
    async fn unsafe_names_table() {
        let names = [
            "".to_string(),
            "/abs.pdf".to_string(),
            "a/../b.pdf".to_string(),
            "a\\b.pdf".to_string(),
            "a\u{0}b.pdf".to_string(),
            "scans/vol1.pdf".to_string(),
        ];
        let files: Vec<Value> = names
            .iter()
            .map(|n| json!({"name": n, "format": "Text PDF"}))
            .collect();
        let body = json!({"metadata": {"identifier": "u"}, "files": files}).to_string();
        let server = TestServer::start(vec![metadata_route("/metadata/u", &body)]);
        let details = local_client()
            .get_item(&local_endpoints(&server), "u")
            .await
            .unwrap();
        // Empty names carry no `.pdf` suffix and no PDF format signal beyond
        // the shared one: every entry here is PDF-ish via `format`.
        assert_eq!(details.pdfs.len(), 6);
        for pdf in &details.pdfs[..5] {
            assert_eq!(
                pdf.availability,
                Availability::Restricted(RestrictedReason::UnsafeName),
                "name {:?} should be unsafe",
                pdf.name
            );
        }
        assert_eq!(
            details.pdfs[5].availability,
            Availability::Available,
            "a normal subfolder name stays available"
        );
    }

    #[tokio::test]
    async fn non_pdf_entries_never_count_toward_cap() {
        let mut files = Vec::new();
        for i in 0..1500 {
            files.push(json!({"name": format!("scan{i}.jp2"), "format": "Single Page JP2"}));
        }
        files.push(json!({"name": "book.pdf", "format": "Text PDF"}));
        let body = json!({"metadata": {"identifier": "n"}, "files": files}).to_string();
        let server = TestServer::start(vec![metadata_route("/metadata/n", &body)]);
        let details = local_client()
            .get_item(&local_endpoints(&server), "n")
            .await
            .unwrap();
        assert_eq!(details.pdfs.len(), 1);
        assert_eq!(details.pdfs[0].name, "book.pdf");
        assert!(!details.truncated);
    }

    #[tokio::test]
    async fn cap_keeps_first_1000_pdfs_in_order() {
        let files: Vec<Value> = (0..1001)
            .map(|i| json!({"name": format!("p{i:04}.pdf"), "format": "Text PDF"}))
            .collect();
        let body = json!({"metadata": {"identifier": "c"}, "files": files}).to_string();
        let server = TestServer::start(vec![metadata_route("/metadata/c", &body)]);
        let details = local_client()
            .get_item(&local_endpoints(&server), "c")
            .await
            .unwrap();
        assert_eq!(details.pdfs.len(), MAX_PDFS_PER_ITEM);
        assert_eq!(details.pdfs.len(), 1000);
        assert!(details.truncated);
        assert_eq!(details.pdfs[0].name, "p0000.pdf");
        assert_eq!(details.pdfs[999].name, "p0999.pdf");
    }

    #[tokio::test]
    async fn empty_object_is_not_found() {
        let server = TestServer::start(vec![
            metadata_route("/metadata/unknownid", "{}"),
            metadata_route("/metadata/nometa", r#"{"files": []}"#),
        ]);
        let endpoints = local_endpoints(&server);
        let client = local_client();
        match client.get_item(&endpoints, "unknownid").await {
            Err(IaError::NotFound { identifier }) => assert_eq!(identifier, "unknownid"),
            other => panic!("expected NotFound, got {other:?}"),
        }
        match client.get_item(&endpoints, "nometa").await {
            Err(IaError::NotFound { .. }) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_identifier_rejected_without_request() {
        let long = "x".repeat(101);
        let bad = vec![
            "".to_string(),
            "has space".to_string(),
            "a/b".to_string(),
            "a*b".to_string(),
            "../evil".to_string(),
            long,
        ];
        let server = TestServer::start(vec![metadata_route("/metadata/ok", "{}")]);
        let endpoints = local_endpoints(&server);
        let client = local_client();
        for id in &bad {
            match client.get_item(&endpoints, id).await {
                Err(IaError::InvalidQuery { .. }) => {}
                other => panic!("{id:?} should be InvalidQuery, got {other:?}"),
            }
        }
        assert!(
            server.targets().is_empty(),
            "no request must be sent, saw {:?}",
            server.targets()
        );
    }

    #[tokio::test]
    async fn request_target_is_exact_metadata_path() {
        let server = TestServer::start(vec![metadata_route(
            &format!("/metadata/{OPEN_ID}"),
            OPEN_FIXTURE,
        )]);
        local_client()
            .get_item(&local_endpoints(&server), OPEN_ID)
            .await
            .unwrap();
        assert_eq!(server.targets(), vec![format!("/metadata/{OPEN_ID}")]);
    }

    #[test]
    fn mediatype_from_open_fixture() {
        let body: Value = serde_json::from_str(OPEN_FIXTURE).unwrap();
        let details = parse_item_value(OPEN_ID, &body).unwrap();
        assert_eq!(details.mediatype.as_deref(), Some("texts"));
        assert!(!details.is_collection());
    }

    #[test]
    fn is_collection_table() {
        for (mediatype, expected) in [
            (Some("collection"), true),
            (Some("Collection"), true),
            (Some("COLLECTION"), true),
            (Some("texts"), false),
            (None, false),
        ] {
            let metadata = match mediatype {
                Some(m) => json!({"identifier": "c", "mediatype": m}),
                None => json!({"identifier": "c"}),
            };
            let details = parse_item_value("c", &json!({"metadata": metadata})).unwrap();
            assert_eq!(details.mediatype.as_deref(), mediatype);
            assert_eq!(
                details.is_collection(),
                expected,
                "mediatype {mediatype:?} should report is_collection() == {expected}"
            );
        }
    }

    #[test]
    fn tolerant_metadata_shapes() {
        let body = json!({
            "metadata": {
                "identifier": "t",
                "title": ["First", "Second"],
                "creator": "Solo",
                "date": 1998,
                "language": "eng",
                "collection": ["a", "b"],
                "rights": true,
                "licenseurl": "https://example.org/license",
            },
            "files": [
                {"name": "noext", "format": "Text PDF", "source": "metadata"},
                {"name": "plain.pdf", "source": "weird"},
            ],
        });
        let details = parse_item_value("t", &body).unwrap();
        assert_eq!(details.title.as_deref(), Some("First"));
        assert_eq!(details.creators, vec!["Solo"]);
        assert_eq!(details.date.as_deref(), Some("1998"));
        assert_eq!(details.languages, vec!["eng"]);
        assert_eq!(details.collections, vec!["a", "b"]);
        assert_eq!(details.rights.as_deref(), Some("true"));
        assert_eq!(
            details.license_url.as_deref(),
            Some("https://example.org/license")
        );
        assert_eq!(details.pdfs.len(), 2);
        assert_eq!(
            details.pdfs[0].source,
            FileSource::Other("metadata".to_string())
        );
        assert_eq!(
            details.pdfs[1].source,
            FileSource::Other("weird".to_string())
        );
        assert_eq!(details.pdfs[1].format, None);
    }

    #[test]
    fn source_and_private_variants() {
        let body = json!({
            "metadata": {"identifier": "v"},
            "files": [
                {"name": "a.pdf", "source": "ORIGINAL"},
                {"name": "b.pdf", "source": "Derivative"},
                {"name": "c.pdf"},
                {"name": "d.PDF", "format": "text pdf", "private": 1},
                {"name": "e.lcpdf", "format": "Something else"},
            ],
        });
        let details = parse_item_value("v", &body).unwrap();
        assert_eq!(details.pdfs.len(), 5);
        assert_eq!(details.pdfs[0].source, FileSource::Original);
        assert_eq!(details.pdfs[1].source, FileSource::Derivative);
        assert_eq!(details.pdfs[2].source, FileSource::Unknown);
        // Case-insensitive format match plus truthy numeric `private`.
        assert_eq!(
            details.pdfs[3].availability,
            Availability::Restricted(RestrictedReason::Private)
        );
        // `.lcpdf` suffix alone is enough for `Encrypted`.
        assert_eq!(
            details.pdfs[4].availability,
            Availability::Restricted(RestrictedReason::Encrypted)
        );
    }
}
