//! Compact provenance record stored under `items.metadata`
//! (`__entropia_internet_archive`, Phase 0 decision D1).
//!
//! The full original IA JSON is never stored here (plan §5 point 1):
//! only the compact value built by [`build_provenance`], capped at
//! [`MAX_PROVENANCE_BYTES`] with marked truncation.

use serde::{Deserialize, Serialize};

use crate::item::{FileSource, ItemDetails};
use crate::job::JobFile;
use crate::prepare::ProvenanceRecord;

/// Reserved key inside `items.metadata`.
pub const PROVENANCE_KEY: &str = "__entropia_internet_archive";
/// Hard cap for the serialized provenance value.
pub const MAX_PROVENANCE_BYTES: usize = 64 * 1024;
/// Schema of the provenance value itself.
const PROVENANCE_SCHEMA: u32 = 1;
/// Provider tag stored in every provenance value.
const PROVIDER: &str = "internet_archive";
/// Marker appended to truncated free-text fields and used as the marker
/// list entry when list fields are shortened.
const TRUNC_MARKER: &str = "…[truncado]";
/// Slack subtracted alongside the excess so JSON escaping overhead cannot
/// keep a truncated value a few bytes over the cap.
const TRUNC_SLACK: usize = 64;

/// Per-item metadata snapshot kept in the job manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemSnapshot {
    pub identifier: String,
    pub title: Option<String>,
    #[serde(default)]
    pub creators: Vec<String>,
    pub date: Option<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub collections: Vec<String>,
    pub rights: Option<String>,
    pub license_url: Option<String>,
}

impl From<&ItemDetails> for ItemSnapshot {
    fn from(details: &ItemDetails) -> Self {
        Self {
            identifier: details.identifier.clone(),
            title: details.title.clone(),
            creators: details.creators.clone(),
            date: details.date.clone(),
            languages: details.languages.clone(),
            collections: details.collections.clone(),
            rights: details.rights.clone(),
            license_url: details.license_url.clone(),
        }
    }
}

/// Error building a provenance value.
#[derive(Debug, thiserror::Error)]
pub enum ProvenanceError {
    #[error("file has no verified sha256 (verify first)")]
    MissingSha256,
    #[error("file has no verified page count (verify first)")]
    MissingPageCount,
    #[error("provenance exceeds 64 KiB even after truncation")]
    TooLarge,
    #[error("invalid url: {0}")]
    Url(#[from] url::ParseError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Identity fields read back from `items.metadata`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceRecordParts {
    pub identifier: String,
    pub file_name: String,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub mtime: Option<u64>,
    pub sha256_local: Option<String>,
    pub page_count: Option<u32>,
}

/// Serializable provenance payload: the VALUE stored under
/// [`PROVENANCE_KEY`]. All keys are snake_case.
#[derive(Debug, Clone, Serialize)]
struct ProvenanceDoc {
    schema: u32,
    provider: String,
    identifier: String,
    file_name: String,
    item_url: String,
    download_url: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mtime: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    md5: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha1: Option<String>,
    sha256_local: String,
    page_count: u32,
    retrieved_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    creators: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    date: Option<String>,
    languages: Vec<String>,
    collections: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rights: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    license_url: Option<String>,
}

fn source_str(source: &FileSource) -> String {
    match source {
        FileSource::Original => "original".to_string(),
        FileSource::Derivative => "derivative".to_string(),
        FileSource::Other(value) => value.clone(),
        FileSource::Unknown => "unknown".to_string(),
    }
}

fn item_url(identifier: &str) -> Result<String, ProvenanceError> {
    let mut url = url::Url::parse("https://archive.org/details")?;
    url.path_segments_mut()
        .map_err(|()| ProvenanceError::Url(url::ParseError::RelativeUrlWithoutBase))?
        .push(identifier);
    Ok(url.to_string())
}

fn download_url(identifier: &str, file_name: &str) -> Result<String, ProvenanceError> {
    let mut url = url::Url::parse("https://archive.org/download")?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|()| ProvenanceError::Url(url::ParseError::RelativeUrlWithoutBase))?;
        segments.push(identifier);
        for part in file_name.split('/') {
            segments.push(part);
        }
    }
    Ok(url.to_string())
}

/// Shorten an optional free-text field so the payload shrinks by at least
/// `excess` bytes. The result ends with [`TRUNC_MARKER`]. Returns `true`
/// when the field actually shrank; tiny fields that would grow by gaining
/// the marker are left alone.
fn truncate_opt_string(field: &mut Option<String>, excess: usize) -> bool {
    let Some(current) = field.as_mut() else {
        return false;
    };
    if current.len() <= TRUNC_MARKER.len() || *current == TRUNC_MARKER {
        return false;
    }
    let new_total = current
        .len()
        .saturating_sub(excess.saturating_add(TRUNC_SLACK));
    let prefix_target = new_total.saturating_sub(TRUNC_MARKER.len());
    let inner = current
        .strip_suffix(TRUNC_MARKER)
        .unwrap_or(current.as_str());
    let mut cut = prefix_target.min(inner.len());
    while cut > 0 && !inner.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut next = inner[..cut].to_string();
    next.push_str(TRUNC_MARKER);
    if next.len() >= current.len() {
        // No progress (overhead rounding): force a halving so the outer
        // loop always terminates.
        let mut half = inner.len() / 2;
        while half > 0 && !inner.is_char_boundary(half) {
            half -= 1;
        }
        next = inner[..half].to_string();
        next.push_str(TRUNC_MARKER);
        if next.len() >= current.len() {
            return false;
        }
    }
    *current = next;
    true
}

/// Drop trailing entries of a list field and leave [`TRUNC_MARKER`] as the
/// last entry. Returns `true` when the list actually shrank.
fn truncate_list(list: &mut Vec<String>, excess: usize) -> bool {
    if list.is_empty() {
        return false;
    }
    if list.len() == 1 && list[0] == TRUNC_MARKER {
        return false;
    }
    let has_marker = list.last().is_some_and(|last| last == TRUNC_MARKER);
    let data_len = if has_marker {
        list.len() - 1
    } else {
        list.len()
    };
    if data_len == 0 {
        return false;
    }
    let data_total: usize = list
        .iter()
        .take(data_len)
        .map(|entry| entry.len().saturating_add(3))
        .sum();
    let avg = (data_total / data_len).max(1);
    let mut to_drop = excess / avg + 1;
    to_drop = to_drop.max(1).min(data_len);
    if has_marker {
        list.pop();
    }
    for _ in 0..to_drop {
        list.pop();
    }
    list.push(TRUNC_MARKER.to_string());
    true
}

/// Build the VALUE stored under [`PROVENANCE_KEY`] (the caller wraps it as
/// `{ PROVENANCE_KEY: value }`).
///
/// # Errors
///
/// Returns [`ProvenanceError::MissingSha256`] or
/// [`ProvenanceError::MissingPageCount`] when the file was not verified
/// first, and [`ProvenanceError::TooLarge`] when the value still exceeds
/// [`MAX_PROVENANCE_BYTES`] after truncating every free-text field.
pub fn build_provenance(
    file: &JobFile,
    item: Option<&ItemSnapshot>,
    retrieved_at: u64,
) -> Result<serde_json::Value, ProvenanceError> {
    let sha256_local = file
        .sha256
        .clone()
        .filter(|hash| !hash.trim().is_empty())
        .ok_or(ProvenanceError::MissingSha256)?;
    let page_count = file.page_count.ok_or(ProvenanceError::MissingPageCount)?;
    let empty = ItemSnapshot {
        identifier: file.identifier.clone(),
        title: None,
        creators: Vec::new(),
        date: None,
        languages: Vec::new(),
        collections: Vec::new(),
        rights: None,
        license_url: None,
    };
    let snapshot = item.unwrap_or(&empty);
    let mut doc = ProvenanceDoc {
        schema: PROVENANCE_SCHEMA,
        provider: PROVIDER.to_string(),
        identifier: file.identifier.clone(),
        file_name: file.file_name.clone(),
        item_url: item_url(&file.identifier)?,
        download_url: download_url(&file.identifier, &file.file_name)?,
        source: source_str(&file.source),
        size: file.expected_size,
        mtime: file.mtime,
        md5: file.md5.clone(),
        sha1: file.sha1.clone(),
        sha256_local,
        page_count,
        retrieved_at,
        title: snapshot.title.clone(),
        creators: snapshot.creators.clone(),
        date: snapshot.date.clone(),
        languages: snapshot.languages.clone(),
        collections: snapshot.collections.clone(),
        rights: snapshot.rights.clone(),
        license_url: snapshot.license_url.clone(),
    };
    if serde_json::to_vec(&doc)?.len() <= MAX_PROVENANCE_BYTES {
        return Ok(serde_json::to_value(&doc)?);
    }
    // Truncate free-text fields in order until the value fits. Identity
    // fields (identifier, file names, checksums, sizes, sha256, page
    // count) are never touched.
    loop {
        let len = serde_json::to_vec(&doc)?.len();
        if len <= MAX_PROVENANCE_BYTES {
            return Ok(serde_json::to_value(&doc)?);
        }
        let excess = len - MAX_PROVENANCE_BYTES;
        if truncate_opt_string(&mut doc.rights, excess) {
            continue;
        }
        if truncate_opt_string(&mut doc.title, excess) {
            continue;
        }
        if truncate_list(&mut doc.creators, excess) {
            continue;
        }
        if truncate_list(&mut doc.collections, excess) {
            continue;
        }
        if truncate_list(&mut doc.languages, excess) {
            continue;
        }
        return Err(ProvenanceError::TooLarge);
    }
}

/// Lenient `u64`: JSON number or decimal string, else `None`.
fn opt_u64(value: Option<&serde_json::Value>) -> Option<u64> {
    match value {
        Some(serde_json::Value::Number(n)) => n.as_u64().or_else(|| {
            n.as_i64().and_then(|i| u64::try_from(i).ok()).or_else(|| {
                n.as_f64().and_then(|f| {
                    if f >= 0.0 && f <= u64::MAX as f64 && f.fract() == 0.0 {
                        Some(f as u64)
                    } else {
                        None
                    }
                })
            })
        }),
        Some(serde_json::Value::String(raw)) => raw.trim().parse::<u64>().ok(),
        _ => None,
    }
}

/// Lenient `u32`: JSON number or decimal string, else `None`.
fn opt_u32(value: Option<&serde_json::Value>) -> Option<u32> {
    opt_u64(value).and_then(|n| u32::try_from(n).ok())
}

fn opt_string(value: Option<&serde_json::Value>) -> Option<String> {
    value.and_then(|v| v.as_str()).map(str::to_string)
}

/// Read `metadata[PROVENANCE_KEY]` from a serialized `items.metadata` JSON
/// object. Tolerant: missing key or wrong types give `None`/`None` fields,
/// never panic.
#[must_use]
pub fn parse_provenance(metadata_json: &str) -> Option<ProvenanceRecordParts> {
    let root: serde_json::Value = serde_json::from_str(metadata_json).ok()?;
    let obj = root.as_object()?.get(PROVENANCE_KEY)?.as_object()?;
    let identifier = obj.get("identifier")?.as_str()?.to_string();
    let file_name = obj.get("file_name")?.as_str()?.to_string();
    Some(ProvenanceRecordParts {
        identifier,
        file_name,
        md5: opt_string(obj.get("md5")),
        sha1: opt_string(obj.get("sha1")),
        size: opt_u64(obj.get("size")),
        mtime: opt_u64(obj.get("mtime")),
        sha256_local: opt_string(obj.get("sha256_local")),
        page_count: opt_u32(obj.get("page_count")),
    })
}

impl ProvenanceRecord {
    /// Build a [`ProvenanceRecord`] from a Corpus row
    /// `(item_id, collection_id, metadata_json)`. Returns `None` when the
    /// metadata has no usable provenance object.
    #[must_use]
    pub fn from_row(
        item_id: &str,
        collection_id: &str,
        metadata_json: &str,
    ) -> Option<ProvenanceRecord> {
        let parts = parse_provenance(metadata_json)?;
        Some(ProvenanceRecord {
            item_id: item_id.to_string(),
            collection_id: collection_id.to_string(),
            identifier: parts.identifier,
            file_name: parts.file_name,
            md5: parts.md5,
            sha1: parts.sha1,
            size: parts.size,
            mtime: parts.mtime,
            sha256_local: parts.sha256_local,
            page_count: parts.page_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Availability, FileSource, PdfFile};
    use crate::job::{FileState, ImportTarget, Job, JobState, JOB_SCHEMA_VERSION};

    const OPEN_FIXTURE: &str = include_str!("../tests/fixtures/metadata-open-text-pdf.json");
    const OPEN_ID: &str = "30-festival-notas-para-una-contra-historia-del-cine-argentino";

    fn available_pdf(name: &str) -> PdfFile {
        PdfFile {
            name: name.to_string(),
            format: Some("Text PDF".to_string()),
            source: FileSource::Original,
            size: Some(1_489_787),
            mtime: Some(1_788_289_643),
            md5: Some("884bfe316f489fd421089ede8afab665".to_string()),
            sha1: Some("8eb49467a31dbc35088346b90c0d381e63fcce56".to_string()),
            availability: Availability::Available,
        }
    }

    fn verified_file(identifier: &str, file_name: &str) -> JobFile {
        let mut f = JobFile::from_pdf(identifier, &available_pdf(file_name)).unwrap();
        f.state = FileState::Verified;
        f.page_count = Some(54);
        f.sha256 = Some("abc123sha256".to_string());
        f
    }

    #[test]
    fn snapshot_from_open_fixture() {
        let body: serde_json::Value = serde_json::from_str(OPEN_FIXTURE).unwrap();
        let details = crate::item::parse_item_value(OPEN_ID, &body).unwrap();
        let snap = ItemSnapshot::from(&details);
        assert_eq!(snap.identifier, OPEN_ID);
        assert_eq!(
            snap.title.as_deref(),
            Some(
                "30º Festival Internacional de Cine de Mar del Plata (2015) - \
                 Notas para una contrahistoria del cine argentino"
            )
        );
        assert!(snap.creators.is_empty());
        assert!(snap.date.is_none());
        assert!(snap.languages.is_empty());
        assert_eq!(snap.collections, vec!["opensource"]);
        assert!(snap.rights.is_none());
        assert!(snap.license_url.is_none());
    }

    #[test]
    fn build_contains_identity_encodes_url_and_requires_sha256() {
        let file = verified_file("my-id", "scans/vol 1/a b.pdf");
        let snap = ItemSnapshot {
            identifier: "my-id".to_string(),
            title: Some("T".to_string()),
            creators: vec!["A".to_string()],
            date: Some("1998".to_string()),
            languages: vec!["eng".to_string()],
            collections: vec!["opensource".to_string()],
            rights: Some("R".to_string()),
            license_url: Some("https://example.org/license".to_string()),
        };
        let value = build_provenance(&file, Some(&snap), 1_700_000_000).unwrap();
        assert_eq!(value.get("schema").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(
            value.get("provider").and_then(|v| v.as_str()),
            Some("internet_archive")
        );
        assert_eq!(
            value.get("identifier").and_then(|v| v.as_str()),
            Some("my-id")
        );
        assert_eq!(
            value.get("file_name").and_then(|v| v.as_str()),
            Some("scans/vol 1/a b.pdf")
        );
        assert_eq!(
            value.get("item_url").and_then(|v| v.as_str()),
            Some("https://archive.org/details/my-id")
        );
        assert_eq!(
            value.get("download_url").and_then(|v| v.as_str()),
            Some("https://archive.org/download/my-id/scans/vol%201/a%20b.pdf")
        );
        assert_eq!(
            value.get("sha256_local").and_then(|v| v.as_str()),
            Some("abc123sha256")
        );
        assert_eq!(value.get("page_count").and_then(|v| v.as_u64()), Some(54));
        assert_eq!(value.get("size").and_then(|v| v.as_u64()), Some(1_489_787));
        assert_eq!(
            value.get("mtime").and_then(|v| v.as_u64()),
            Some(1_788_289_643)
        );
        assert_eq!(
            value.get("md5").and_then(|v| v.as_str()),
            Some("884bfe316f489fd421089ede8afab665")
        );
        assert_eq!(
            value.get("sha1").and_then(|v| v.as_str()),
            Some("8eb49467a31dbc35088346b90c0d381e63fcce56")
        );
        assert_eq!(
            value.get("retrieved_at").and_then(|v| v.as_u64()),
            Some(1_700_000_000)
        );
        assert_eq!(value.get("title").and_then(|v| v.as_str()), Some("T"));

        let mut no_hash = file.clone();
        no_hash.sha256 = None;
        assert!(matches!(
            build_provenance(&no_hash, Some(&snap), 1),
            Err(ProvenanceError::MissingSha256)
        ));
        let mut no_pages = file.clone();
        no_pages.page_count = None;
        assert!(matches!(
            build_provenance(&no_pages, Some(&snap), 1),
            Err(ProvenanceError::MissingPageCount)
        ));
    }

    #[test]
    fn cap_truncates_huge_title_and_creators() {
        let file = verified_file("big-id", "big.pdf");
        let snap = ItemSnapshot {
            identifier: "big-id".to_string(),
            title: Some("x".repeat(200 * 1024)),
            creators: (0..10_000).map(|i| format!("creator-{i:05}")).collect(),
            date: None,
            languages: vec![],
            collections: vec![],
            rights: None,
            license_url: None,
        };
        let value = build_provenance(&file, Some(&snap), 7).unwrap();
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(
            bytes.len() <= MAX_PROVENANCE_BYTES,
            "provenance is {} bytes, over 64 KiB",
            bytes.len()
        );
        // Identity untouched.
        assert_eq!(
            value.get("identifier").and_then(|v| v.as_str()),
            Some("big-id")
        );
        assert_eq!(
            value.get("file_name").and_then(|v| v.as_str()),
            Some("big.pdf")
        );
        assert_eq!(
            value.get("sha256_local").and_then(|v| v.as_str()),
            Some("abc123sha256")
        );
        assert_eq!(value.get("page_count").and_then(|v| v.as_u64()), Some(54));
        let title = value.get("title").and_then(|v| v.as_str()).unwrap();
        assert!(
            title.ends_with("…[truncado]"),
            "title should end with marker, got tail {:?}",
            &title[title.len().saturating_sub(20)..]
        );
    }

    #[test]
    fn round_trip_through_from_row() {
        let file = verified_file("rt-id", "doc.pdf");
        let snap = ItemSnapshot {
            identifier: "rt-id".to_string(),
            title: Some("Title".to_string()),
            creators: vec!["Crea".to_string()],
            date: Some("2001".to_string()),
            languages: vec!["spa".to_string()],
            collections: vec!["opensource".to_string()],
            rights: None,
            license_url: None,
        };
        let value = build_provenance(&file, Some(&snap), 42).unwrap();
        let metadata = serde_json::json!({ PROVENANCE_KEY: value }).to_string();
        let record = ProvenanceRecord::from_row("item-1", "col-1", &metadata).expect("round trip");
        assert_eq!(record.item_id, "item-1");
        assert_eq!(record.collection_id, "col-1");
        assert_eq!(record.identifier, "rt-id");
        assert_eq!(record.file_name, "doc.pdf");
        assert_eq!(
            record.md5.as_deref(),
            Some("884bfe316f489fd421089ede8afab665")
        );
        assert_eq!(
            record.sha1.as_deref(),
            Some("8eb49467a31dbc35088346b90c0d381e63fcce56")
        );
        assert_eq!(record.size, Some(1_489_787));
        assert_eq!(record.mtime, Some(1_788_289_643));
        assert_eq!(record.sha256_local.as_deref(), Some("abc123sha256"));
        assert_eq!(record.page_count, Some(54));
    }

    #[test]
    fn from_row_none_cases() {
        assert!(ProvenanceRecord::from_row("i", "c", "{}").is_none());
        assert!(ProvenanceRecord::from_row("i", "c", r#"{"other":1}"#).is_none());
        assert!(ProvenanceRecord::from_row("i", "c", "{ not json").is_none());
        let s = serde_json::json!({ PROVENANCE_KEY: "string" }).to_string();
        assert!(ProvenanceRecord::from_row("i", "c", &s).is_none());
        let n = serde_json::json!({ PROVENANCE_KEY: 42 }).to_string();
        assert!(ProvenanceRecord::from_row("i", "c", &n).is_none());
        let a = serde_json::json!({ PROVENANCE_KEY: [] }).to_string();
        assert!(ProvenanceRecord::from_row("i", "c", &a).is_none());
        let null = serde_json::json!({ PROVENANCE_KEY: null }).to_string();
        assert!(ProvenanceRecord::from_row("i", "c", &null).is_none());
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        let file = verified_file("uni-id", "uni.pdf");
        let snap = ItemSnapshot {
            identifier: "uni-id".to_string(),
            // Multi-byte chars: a naive byte cut would split them.
            title: Some("é".repeat(100 * 1024)),
            creators: vec!["ñ".repeat(100 * 1024)],
            date: None,
            languages: vec![],
            collections: vec![],
            rights: Some("ü".repeat(100 * 1024)),
            license_url: None,
        };
        let value = build_provenance(&file, Some(&snap), 7).unwrap();
        let bytes = serde_json::to_vec(&value).unwrap();
        assert!(bytes.len() <= MAX_PROVENANCE_BYTES);
        // Serialization round trip proves valid UTF-8 on char boundaries.
        let back: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(back
            .get("title")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t.ends_with("…[truncado]")));
    }

    #[test]
    fn pathological_identity_overflow_errors() {
        // Identity fields are never truncated: a 70 KB file name cannot fit.
        let file = verified_file("huge-id", &"y".repeat(70 * 1024));
        assert!(matches!(
            build_provenance(&file, None, 7),
            Err(ProvenanceError::TooLarge)
        ));
    }

    #[test]
    fn parse_tolerates_wrong_field_types() {
        let metadata = serde_json::json!({
            PROVENANCE_KEY: {
                "identifier": "w-id",
                "file_name": "w.pdf",
                "md5": 42,
                "sha1": ["x"],
                "size": "not-a-number",
                "mtime": true,
                "sha256_local": "abc",
                "page_count": "54",
            }
        })
        .to_string();
        let record = ProvenanceRecord::from_row("i", "c", &metadata).expect("tolerant");
        assert_eq!(record.identifier, "w-id");
        assert_eq!(record.file_name, "w.pdf");
        assert_eq!(record.md5, None);
        assert_eq!(record.sha1, None);
        assert_eq!(record.size, None);
        assert_eq!(record.mtime, None);
        assert_eq!(record.sha256_local.as_deref(), Some("abc"));
        assert_eq!(record.page_count, Some(54));
    }

    #[test]
    fn old_job_json_without_items_loads_empty() {
        let json = serde_json::json!({
            "schema_version": JOB_SCHEMA_VERSION,
            "id": "12345678-1234-1234-1234-1234567890ab",
            "created_at": 1,
            "updated_at": 1,
            "target": { "existing_collection": { "collection_id": "c" } },
            "state": "draft",
            "files": [],
        })
        .to_string();
        let job: Job = serde_json::from_str(&json).unwrap();
        assert!(job.items.is_empty());
        assert!(job.item("anything").is_none());

        let mut with_items = job.clone();
        with_items.items = vec![ItemSnapshot {
            identifier: "a".to_string(),
            title: None,
            creators: vec![],
            date: None,
            languages: vec![],
            collections: vec![],
            rights: None,
            license_url: None,
        }];
        assert_eq!(with_items.item("a").unwrap().identifier, "a");
        assert!(with_items.item("b").is_none());
        let _ = JobState::Draft;
        let _ = ImportTarget::ExistingCollection {
            collection_id: "c".to_string(),
        };
    }
}
