//! PURE preparation summary (Phase 2 task 15).
//!
//! No filesystem, no network, no clock. The app owns provenance lookup
//! (SQLite over `items.metadata`) and implements [`ProvenanceLookup`];
//! this module only classifies candidates against those records.
//!
//! Identity (plan §5): remote key `(provider, identifier, file_name)` plus
//! a remote version fingerprint — `md5`, else `sha1`, else `size`+`mtime`.
//! Dedup consults provenance; the MVP never shares one binary across
//! collections, so an `InOtherCollection` match is reported, never linked.

use crate::item::{Availability, PdfFile};
use crate::job::{ImportTarget, JobFile};

/// Remote version fingerprint, strongest available signal first.
///
/// Hex strings compare case-insensitively and are stored lowercase;
/// non-hex or empty values count as missing and fall through to the next
/// priority. `size`+`mtime` only counts when both are present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteFingerprint {
    Md5(String),
    Sha1(String),
    SizeMtime { size: u64, mtime: u64 },
    None,
}

impl RemoteFingerprint {
    /// Strongest fingerprint of a [`PdfFile`]: md5, else sha1, else
    /// size+mtime, else [`RemoteFingerprint::None`].
    #[must_use]
    pub fn from_pdf(pdf: &PdfFile) -> Self {
        if let Some(md5) = pdf.md5.as_deref().and_then(normalize_hex) {
            return Self::Md5(md5);
        }
        if let Some(sha1) = pdf.sha1.as_deref().and_then(normalize_hex) {
            return Self::Sha1(sha1);
        }
        if let (Some(size), Some(mtime)) = (pdf.size, pdf.mtime) {
            return Self::SizeMtime { size, mtime };
        }
        Self::None
    }

    /// Strongest fingerprint of a [`JobFile`]: md5, else sha1, else
    /// `expected_size`+`mtime`, else [`RemoteFingerprint::None`].
    #[must_use]
    pub fn from_job_file(file: &JobFile) -> Self {
        if let Some(md5) = file.md5.as_deref().and_then(normalize_hex) {
            return Self::Md5(md5);
        }
        if let Some(sha1) = file.sha1.as_deref().and_then(normalize_hex) {
            return Self::Sha1(sha1);
        }
        if let (Some(size), Some(mtime)) = (file.expected_size, file.mtime) {
            return Self::SizeMtime { size, mtime };
        }
        Self::None
    }

    /// Strongest fingerprint of a stored [`ProvenanceRecord`].
    #[must_use]
    pub fn from_record(record: &ProvenanceRecord) -> Self {
        if let Some(md5) = record.md5.as_deref().and_then(normalize_hex) {
            return Self::Md5(md5);
        }
        if let Some(sha1) = record.sha1.as_deref().and_then(normalize_hex) {
            return Self::Sha1(sha1);
        }
        if let (Some(size), Some(mtime)) = (record.size, record.mtime) {
            return Self::SizeMtime { size, mtime };
        }
        Self::None
    }

    /// True unless the fingerprint carries no comparable signal.
    #[must_use]
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

impl From<&PdfFile> for RemoteFingerprint {
    fn from(pdf: &PdfFile) -> Self {
        Self::from_pdf(pdf)
    }
}

impl From<&JobFile> for RemoteFingerprint {
    fn from(file: &JobFile) -> Self {
        Self::from_job_file(file)
    }
}

impl From<&ProvenanceRecord> for RemoteFingerprint {
    fn from(record: &ProvenanceRecord) -> Self {
        Self::from_record(record)
    }
}

/// Trimmed non-empty ASCII-hex, lowercased; anything else is missing.
fn normalize_hex(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if !trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

/// Normalized version fields backing [`same_version`].
///
/// Implemented for [`PdfFile`], [`JobFile`], [`ProvenanceRecord`] and
/// [`RemoteFingerprint`] so `same_version` accepts any candidate shape:
/// a bare fingerprint only carries its own kind, while the full structs
/// carry every field for strongest-common-kind fallback.
pub trait FingerprintSource {
    fn fingerprint_md5(&self) -> Option<String>;
    fn fingerprint_sha1(&self) -> Option<String>;
    fn fingerprint_size(&self) -> Option<u64>;
    fn fingerprint_mtime(&self) -> Option<u64>;
}

impl FingerprintSource for PdfFile {
    fn fingerprint_md5(&self) -> Option<String> {
        self.md5.as_deref().and_then(normalize_hex)
    }
    fn fingerprint_sha1(&self) -> Option<String> {
        self.sha1.as_deref().and_then(normalize_hex)
    }
    fn fingerprint_size(&self) -> Option<u64> {
        self.size
    }
    fn fingerprint_mtime(&self) -> Option<u64> {
        self.mtime
    }
}

impl FingerprintSource for JobFile {
    fn fingerprint_md5(&self) -> Option<String> {
        self.md5.as_deref().and_then(normalize_hex)
    }
    fn fingerprint_sha1(&self) -> Option<String> {
        self.sha1.as_deref().and_then(normalize_hex)
    }
    fn fingerprint_size(&self) -> Option<u64> {
        self.expected_size
    }
    fn fingerprint_mtime(&self) -> Option<u64> {
        self.mtime
    }
}

impl FingerprintSource for ProvenanceRecord {
    fn fingerprint_md5(&self) -> Option<String> {
        self.md5.as_deref().and_then(normalize_hex)
    }
    fn fingerprint_sha1(&self) -> Option<String> {
        self.sha1.as_deref().and_then(normalize_hex)
    }
    fn fingerprint_size(&self) -> Option<u64> {
        self.size
    }
    fn fingerprint_mtime(&self) -> Option<u64> {
        self.mtime
    }
}

impl FingerprintSource for RemoteFingerprint {
    fn fingerprint_md5(&self) -> Option<String> {
        match self {
            Self::Md5(value) => Some(value.clone()),
            _ => None,
        }
    }
    fn fingerprint_sha1(&self) -> Option<String> {
        match self {
            Self::Sha1(value) => Some(value.clone()),
            _ => None,
        }
    }
    fn fingerprint_size(&self) -> Option<u64> {
        match self {
            Self::SizeMtime { size, .. } => Some(*size),
            _ => None,
        }
    }
    fn fingerprint_mtime(&self) -> Option<u64> {
        match self {
            Self::SizeMtime { mtime, .. } => Some(*mtime),
            _ => None,
        }
    }
}

/// Stored provenance row for one `(identifier, file_name)` import.
///
/// The app fills this from `items.metadata`; `item_id` is the local Corpus
/// item, `collection_id` its owning collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceRecord {
    pub item_id: String,
    pub collection_id: String,
    pub identifier: String,
    pub file_name: String,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub mtime: Option<u64>,
}

impl ProvenanceRecord {
    /// Strongest fingerprint held by this record.
    #[must_use]
    pub fn fingerprint(&self) -> RemoteFingerprint {
        RemoteFingerprint::from_record(self)
    }
}

/// Compare a candidate against one stored record by the strongest kind
/// both sides hold: md5 when both have a valid md5, else sha1 when both
/// have one, else size+mtime when both sides hold both numbers.
///
/// Returns `Some(true)` on equal values, `Some(false)` on different
/// values, and `None` when no kind is comparable on both sides.
#[must_use]
pub fn same_version(candidate: &impl FingerprintSource, record: &ProvenanceRecord) -> Option<bool> {
    if let (Some(left), Some(right)) = (candidate.fingerprint_md5(), record.fingerprint_md5()) {
        return Some(left == right);
    }
    if let (Some(left), Some(right)) = (candidate.fingerprint_sha1(), record.fingerprint_sha1()) {
        return Some(left == right);
    }
    if let (Some(lsize), Some(lmtime), Some(rsize), Some(rmtime)) = (
        candidate.fingerprint_size(),
        candidate.fingerprint_mtime(),
        record.fingerprint_size(),
        record.fingerprint_mtime(),
    ) {
        return Some(lsize == rsize && lmtime == rmtime);
    }
    None
}

/// Provenance index owned by the app (SQLite later). Returns every record
/// for `(identifier, file_name)` across all collections.
pub trait ProvenanceLookup {
    fn find(&self, identifier: &str, file_name: &str) -> Vec<ProvenanceRecord>;
}

/// Dedup outcome for one candidate file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DuplicateStatus {
    /// No provenance record for this `(identifier, file_name)`.
    New,
    /// A record lives in the target collection with the same version
    /// (`Some(true)`) or with no comparable fingerprint (`None`,
    /// conservative: treat as already imported).
    AlreadyInTarget { item_id: String },
    /// A record lives in the target collection with a different version
    /// (`Some(false)`): import creates a separate document, never an
    /// overwrite.
    NewVersion { previous_item_id: String },
    /// Same-version (or unknown-version, conservative) records exist only
    /// in other collections. The MVP never shares the binary: the caller
    /// offers omit-or-independent-copy.
    InOtherCollection {
        item_id: String,
        collection_id: String,
    },
}

/// Classify one available [`PdfFile`].
///
/// Records are sorted by `(item_id, collection_id)` first so the answer is
/// deterministic. Precedence: `AlreadyInTarget` > `NewVersion` >
/// `InOtherCollection` > `New`. For [`ImportTarget::NewCollection`] every
/// match counts as `InOtherCollection` since the destination has no id yet.
#[must_use]
pub fn classify(
    candidate: &PdfFile,
    identifier: &str,
    target: &ImportTarget,
    lookup: &dyn ProvenanceLookup,
) -> DuplicateStatus {
    let mut records = lookup.find(identifier, &candidate.name);
    records.sort_by(|a, b| {
        a.item_id
            .cmp(&b.item_id)
            .then_with(|| a.collection_id.cmp(&b.collection_id))
    });
    match target {
        ImportTarget::ExistingCollection { collection_id } => {
            for record in &records {
                if &record.collection_id == collection_id
                    && matches!(same_version(candidate, record), Some(true) | None)
                {
                    return DuplicateStatus::AlreadyInTarget {
                        item_id: record.item_id.clone(),
                    };
                }
            }
            if let Some(record) = records
                .iter()
                .find(|record| &record.collection_id == collection_id)
            {
                return DuplicateStatus::NewVersion {
                    previous_item_id: record.item_id.clone(),
                };
            }
            for record in &records {
                if &record.collection_id != collection_id
                    && matches!(same_version(candidate, record), Some(true) | None)
                {
                    return DuplicateStatus::InOtherCollection {
                        item_id: record.item_id.clone(),
                        collection_id: record.collection_id.clone(),
                    };
                }
            }
            DuplicateStatus::New
        }
        ImportTarget::NewCollection { .. } => {
            for record in &records {
                if matches!(same_version(candidate, record), Some(true) | None) {
                    return DuplicateStatus::InOtherCollection {
                        item_id: record.item_id.clone(),
                        collection_id: record.collection_id.clone(),
                    };
                }
            }
            DuplicateStatus::New
        }
    }
}

/// One row of the preparation table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareEntry {
    pub identifier: String,
    pub file_name: String,
    pub availability: Availability,
    pub size: Option<u64>,
    /// `None` for restricted files (never classified, never counted).
    pub duplicate: Option<DuplicateStatus>,
}

/// Counts and rows for the preparation screen (plan §8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareSummary {
    pub total_files: usize,
    pub available_files: usize,
    pub restricted_files: usize,
    /// Saturating sum of known sizes of available files only.
    pub known_bytes: u64,
    /// Available files with unknown (`None`) size.
    pub unknown_size_files: usize,
    pub new_files: usize,
    pub already_in_target: usize,
    pub in_other_collection: usize,
    pub new_version: usize,
    pub entries: Vec<PrepareEntry>,
}

impl PrepareSummary {
    /// Synced-volume estimate (plan §5 "Volumen sincronizado"): the original
    /// plus its split pages, so known bytes times two, saturating.
    ///
    /// Counts only files that would actually be imported (`New` and
    /// `NewVersion`); restricted, already-imported and undecided
    /// `InOtherCollection` files contribute nothing. Use
    /// [`PrepareSummary::in_other_collection_bytes`] for the undecided pile.
    #[must_use]
    pub fn estimated_sync_bytes(&self) -> u64 {
        let mut total: u64 = 0;
        for entry in &self.entries {
            let counted = matches!(
                entry.duplicate,
                Some(DuplicateStatus::New) | Some(DuplicateStatus::NewVersion { .. })
            );
            if counted {
                total = total.saturating_add(entry.size.unwrap_or(0));
            }
        }
        total.saturating_mul(2)
    }

    /// Same doubling estimate restricted to the `InOtherCollection` pile,
    /// so the UI can show "plus X if you also copy those".
    #[must_use]
    pub fn in_other_collection_bytes(&self) -> u64 {
        let mut total: u64 = 0;
        for entry in &self.entries {
            if matches!(
                entry.duplicate,
                Some(DuplicateStatus::InOtherCollection { .. })
            ) {
                total = total.saturating_add(entry.size.unwrap_or(0));
            }
        }
        total.saturating_mul(2)
    }
}

/// Build the preparation summary preserving input order.
///
/// Restricted files get `duplicate: None` and contribute to neither
/// `known_bytes`/`unknown_size_files` nor any duplicate counter.
#[must_use]
pub fn summarize(
    files: &[(String, PdfFile)],
    target: &ImportTarget,
    lookup: &dyn ProvenanceLookup,
) -> PrepareSummary {
    let mut summary = PrepareSummary {
        total_files: files.len(),
        available_files: 0,
        restricted_files: 0,
        known_bytes: 0,
        unknown_size_files: 0,
        new_files: 0,
        already_in_target: 0,
        in_other_collection: 0,
        new_version: 0,
        entries: Vec::with_capacity(files.len()),
    };
    for (identifier, pdf) in files {
        if pdf.availability != Availability::Available {
            summary.restricted_files += 1;
            summary.entries.push(PrepareEntry {
                identifier: identifier.clone(),
                file_name: pdf.name.clone(),
                availability: pdf.availability.clone(),
                size: pdf.size,
                duplicate: None,
            });
            continue;
        }
        summary.available_files += 1;
        match pdf.size {
            Some(size) => {
                summary.known_bytes = summary.known_bytes.saturating_add(size);
            }
            None => {
                summary.unknown_size_files += 1;
            }
        }
        let status = classify(pdf, identifier, target, lookup);
        match &status {
            DuplicateStatus::New => summary.new_files += 1,
            DuplicateStatus::AlreadyInTarget { .. } => summary.already_in_target += 1,
            DuplicateStatus::InOtherCollection { .. } => summary.in_other_collection += 1,
            DuplicateStatus::NewVersion { .. } => summary.new_version += 1,
        }
        summary.entries.push(PrepareEntry {
            identifier: identifier.clone(),
            file_name: pdf.name.clone(),
            availability: pdf.availability.clone(),
            size: pdf.size,
            duplicate: Some(status),
        });
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{FileSource, RestrictedReason};
    use crate::job::JobFile;

    fn avail_pdf(
        name: &str,
        md5: Option<&str>,
        sha1: Option<&str>,
        size: Option<u64>,
        mtime: Option<u64>,
    ) -> PdfFile {
        PdfFile {
            name: name.to_string(),
            format: Some("Text PDF".to_string()),
            source: FileSource::Original,
            size,
            mtime,
            md5: md5.map(str::to_string),
            sha1: sha1.map(str::to_string),
            availability: Availability::Available,
        }
    }

    fn restricted_pdf(name: &str, reason: RestrictedReason) -> PdfFile {
        PdfFile {
            name: name.to_string(),
            format: Some("Text PDF".to_string()),
            source: FileSource::Original,
            size: Some(999),
            mtime: Some(111),
            md5: Some("abc".to_string()),
            sha1: None,
            availability: Availability::Restricted(reason),
        }
    }

    fn record(
        item_id: &str,
        collection_id: &str,
        md5: Option<&str>,
        sha1: Option<&str>,
        size: Option<u64>,
        mtime: Option<u64>,
    ) -> ProvenanceRecord {
        ProvenanceRecord {
            item_id: item_id.to_string(),
            collection_id: collection_id.to_string(),
            identifier: "ident".to_string(),
            file_name: "doc.pdf".to_string(),
            md5: md5.map(str::to_string),
            sha1: sha1.map(str::to_string),
            size,
            mtime,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record_named(
        item_id: &str,
        collection_id: &str,
        identifier: &str,
        file_name: &str,
        md5: Option<&str>,
        sha1: Option<&str>,
        size: Option<u64>,
        mtime: Option<u64>,
    ) -> ProvenanceRecord {
        ProvenanceRecord {
            item_id: item_id.to_string(),
            collection_id: collection_id.to_string(),
            identifier: identifier.to_string(),
            file_name: file_name.to_string(),
            md5: md5.map(str::to_string),
            sha1: sha1.map(str::to_string),
            size,
            mtime,
        }
    }

    /// In-memory lookup filtering by `(identifier, file_name)` like the
    /// real SQLite lookup will.
    struct MemLookup(Vec<ProvenanceRecord>);

    impl ProvenanceLookup for MemLookup {
        fn find(&self, identifier: &str, file_name: &str) -> Vec<ProvenanceRecord> {
            self.0
                .iter()
                .filter(|r| r.identifier == identifier && r.file_name == file_name)
                .cloned()
                .collect()
        }
    }

    fn existing(id: &str) -> ImportTarget {
        ImportTarget::ExistingCollection {
            collection_id: id.to_string(),
        }
    }

    fn new_collection() -> ImportTarget {
        ImportTarget::NewCollection {
            name: "Nueva".to_string(),
        }
    }

    // ---- RemoteFingerprint priority ----

    #[test]
    fn fingerprint_prefers_md5_over_sha1_over_sizemtime() {
        let full = avail_pdf("a.pdf", Some("aa"), Some("bb"), Some(10), Some(20));
        assert_eq!(
            RemoteFingerprint::from_pdf(&full),
            RemoteFingerprint::Md5("aa".to_string())
        );

        let no_md5 = avail_pdf("a.pdf", None, Some("bb"), Some(10), Some(20));
        assert_eq!(
            RemoteFingerprint::from_pdf(&no_md5),
            RemoteFingerprint::Sha1("bb".to_string())
        );

        let size_only = avail_pdf("a.pdf", None, None, Some(10), Some(20));
        assert_eq!(
            RemoteFingerprint::from_pdf(&size_only),
            RemoteFingerprint::SizeMtime {
                size: 10,
                mtime: 20
            }
        );
    }

    #[test]
    fn fingerprint_none_when_nothing_comparable() {
        for pdf in [
            avail_pdf("a.pdf", None, None, None, None),
            avail_pdf("a.pdf", None, None, Some(10), None),
            avail_pdf("a.pdf", None, None, None, Some(20)),
            avail_pdf("a.pdf", Some(""), None, None, None),
            avail_pdf("a.pdf", Some("zz-not-hex"), Some(""), None, None),
        ] {
            assert_eq!(RemoteFingerprint::from_pdf(&pdf), RemoteFingerprint::None);
        }
    }

    #[test]
    fn fingerprint_hex_case_insensitive_and_lowercased() {
        let upper = avail_pdf("a.pdf", Some("ABCDEF0123"), None, None, None);
        assert_eq!(
            RemoteFingerprint::from_pdf(&upper),
            RemoteFingerprint::Md5("abcdef0123".to_string())
        );
        let mixed = avail_pdf("a.pdf", None, Some("AbCdEf"), None, None);
        assert_eq!(
            RemoteFingerprint::from_pdf(&mixed),
            RemoteFingerprint::Sha1("abcdef".to_string())
        );
        // Whitespace-padded hex still counts after trimming.
        let padded = avail_pdf("a.pdf", Some("  AB12  "), None, None, None);
        assert_eq!(
            RemoteFingerprint::from_pdf(&padded),
            RemoteFingerprint::Md5("ab12".to_string())
        );
    }

    #[test]
    fn fingerprint_invalid_hex_falls_through() {
        // Non-hex md5 is ignored, valid sha1 wins.
        let pdf = avail_pdf("a.pdf", Some("not-hex!!"), Some("beef"), Some(5), Some(6));
        assert_eq!(
            RemoteFingerprint::from_pdf(&pdf),
            RemoteFingerprint::Sha1("beef".to_string())
        );
        // Both hashes invalid: size+mtime wins.
        let pdf = avail_pdf("a.pdf", Some("xx"), Some("yy"), Some(5), Some(6));
        assert_eq!(
            RemoteFingerprint::from_pdf(&pdf),
            RemoteFingerprint::SizeMtime { size: 5, mtime: 6 }
        );
    }

    #[test]
    fn fingerprint_from_job_file_uses_expected_size() {
        let pdf = avail_pdf("a.pdf", Some("AA"), Some("BB"), Some(7), Some(8));
        let job = JobFile::from_pdf("ident", &pdf).expect("available");
        assert_eq!(
            RemoteFingerprint::from_job_file(&job),
            RemoteFingerprint::Md5("aa".to_string())
        );
        let mut sha_only = job.clone();
        sha_only.md5 = None;
        assert_eq!(
            RemoteFingerprint::from_job_file(&sha_only),
            RemoteFingerprint::Sha1("bb".to_string())
        );
        let mut sized = job.clone();
        sized.md5 = None;
        sized.sha1 = None;
        assert_eq!(
            RemoteFingerprint::from_job_file(&sized),
            RemoteFingerprint::SizeMtime { size: 7, mtime: 8 }
        );
        let mut nothing = sized.clone();
        nothing.expected_size = None;
        assert_eq!(
            RemoteFingerprint::from_job_file(&nothing),
            RemoteFingerprint::None
        );
    }

    // ---- same_version ----

    #[test]
    fn same_version_md5_table_case_insensitive() {
        let candidate = avail_pdf("doc.pdf", Some("ABC"), None, Some(1), Some(2));
        assert_eq!(
            same_version(
                &candidate,
                &record("i", "c", Some("abc"), None, Some(1), Some(2))
            ),
            Some(true)
        );
        // Different md5 wins even when everything else matches.
        assert_eq!(
            same_version(
                &candidate,
                &record("i", "c", Some("def"), Some(""), Some(1), Some(2))
            ),
            Some(false)
        );
    }

    #[test]
    fn same_version_sha1_comparison() {
        let candidate = avail_pdf("doc.pdf", None, Some("a1"), Some(1), Some(2));
        assert_eq!(
            same_version(&candidate, &record("i", "c", None, Some("A1"), None, None)),
            Some(true)
        );
        assert_eq!(
            same_version(&candidate, &record("i", "c", None, Some("b2"), None, None)),
            Some(false)
        );
    }

    #[test]
    fn same_version_size_mtime_comparison() {
        let candidate = avail_pdf("doc.pdf", None, None, Some(100), Some(200));
        assert_eq!(
            same_version(
                &candidate,
                &record("i", "c", None, None, Some(100), Some(200))
            ),
            Some(true)
        );
        assert_eq!(
            same_version(
                &candidate,
                &record("i", "c", None, None, Some(101), Some(200))
            ),
            Some(false)
        );
        assert_eq!(
            same_version(
                &candidate,
                &record("i", "c", None, None, Some(100), Some(201))
            ),
            Some(false)
        );
    }

    #[test]
    fn same_version_md5_vs_sha1_only_falls_back_to_size_mtime() {
        // Candidate holds md5 (+size+mtime, no sha1); record holds sha1
        // (+size+mtime, no md5): no common hash, so size+mtime decides.
        let candidate = avail_pdf("doc.pdf", Some("aa"), None, Some(10), Some(20));
        let same = record("i", "c", None, Some("bb"), Some(10), Some(20));
        assert_eq!(same_version(&candidate, &same), Some(true));
        let different = record("i", "c", None, Some("bb"), Some(10), Some(21));
        assert_eq!(same_version(&candidate, &different), Some(false));
    }

    #[test]
    fn same_version_none_when_nothing_comparable() {
        // md5-only candidate vs size+mtime-only record: no shared kind.
        let candidate = avail_pdf("doc.pdf", Some("aa"), None, None, None);
        let sized = record("i", "c", None, None, Some(1), Some(2));
        assert_eq!(same_version(&candidate, &sized), None);
        // md5 vs sha1 with no sizes on either side.
        let sha_only = record("i", "c", None, Some("bb"), None, None);
        assert_eq!(same_version(&candidate, &sha_only), None);
        // Fingerprint-less candidate never compares.
        let empty = avail_pdf("doc.pdf", None, None, None, None);
        let full = record("i", "c", Some("aa"), Some("bb"), Some(1), Some(2));
        assert_eq!(same_version(&empty, &full), None);
        // Half size+mtime (size without mtime) is not comparable.
        let half = avail_pdf("doc.pdf", None, None, Some(1), None);
        assert_eq!(same_version(&half, &sized), None);
    }

    #[test]
    fn same_version_accepts_job_and_fingerprint_candidates() {
        let pdf = avail_pdf("doc.pdf", Some("AA"), None, Some(3), Some(4));
        let job = JobFile::from_pdf("ident", &pdf).expect("available");
        let rec = record("i", "c", Some("aa"), None, None, None);
        assert_eq!(same_version(&job, &rec), Some(true));
        let fp = RemoteFingerprint::from_pdf(&pdf);
        assert_eq!(same_version(&fp, &rec), Some(true));
    }

    // ---- classify ----

    #[test]
    fn classify_new_when_no_record() {
        let candidate = avail_pdf("doc.pdf", Some("aa"), None, Some(1), Some(2));
        let lookup = MemLookup(vec![]);
        assert_eq!(
            classify(&candidate, "ident", &existing("c1"), &lookup),
            DuplicateStatus::New
        );
        assert_eq!(
            classify(&candidate, "ident", &new_collection(), &lookup),
            DuplicateStatus::New
        );
    }

    #[test]
    fn classify_already_in_target_on_same_version() {
        let candidate = avail_pdf("doc.pdf", Some("aa"), None, Some(1), Some(2));
        let lookup = MemLookup(vec![record(
            "item-1",
            "c1",
            Some("aa"),
            None,
            Some(1),
            Some(2),
        )]);
        assert_eq!(
            classify(&candidate, "ident", &existing("c1"), &lookup),
            DuplicateStatus::AlreadyInTarget {
                item_id: "item-1".to_string()
            }
        );
    }

    #[test]
    fn classify_already_in_target_when_no_comparable_data() {
        // Conservative: unknown version in the target counts as imported.
        let candidate = avail_pdf("doc.pdf", Some("aa"), None, None, None);
        let lookup = MemLookup(vec![record("item-1", "c1", None, None, Some(1), Some(2))]);
        assert_eq!(same_version(&candidate, &lookup.0[0]), None);
        assert_eq!(
            classify(&candidate, "ident", &existing("c1"), &lookup),
            DuplicateStatus::AlreadyInTarget {
                item_id: "item-1".to_string()
            }
        );
    }

    #[test]
    fn classify_new_version_on_different_version_in_target() {
        let candidate = avail_pdf("doc.pdf", Some("aa"), None, Some(1), Some(2));
        let lookup = MemLookup(vec![record(
            "item-1",
            "c1",
            Some("bb"),
            None,
            Some(1),
            Some(2),
        )]);
        assert_eq!(
            classify(&candidate, "ident", &existing("c1"), &lookup),
            DuplicateStatus::NewVersion {
                previous_item_id: "item-1".to_string()
            }
        );
    }

    #[test]
    fn classify_in_other_collection_only() {
        let candidate = avail_pdf("doc.pdf", Some("aa"), None, Some(1), Some(2));
        let lookup = MemLookup(vec![record(
            "item-9",
            "other",
            Some("aa"),
            None,
            Some(1),
            Some(2),
        )]);
        assert_eq!(
            classify(&candidate, "ident", &existing("c1"), &lookup),
            DuplicateStatus::InOtherCollection {
                item_id: "item-9".to_string(),
                collection_id: "other".to_string(),
            }
        );
    }

    #[test]
    fn classify_new_collection_every_match_is_other() {
        let candidate = avail_pdf("doc.pdf", Some("aa"), None, Some(1), Some(2));
        let lookup = MemLookup(vec![record(
            "item-1",
            "c1",
            Some("aa"),
            None,
            Some(1),
            Some(2),
        )]);
        assert_eq!(
            classify(&candidate, "ident", &new_collection(), &lookup),
            DuplicateStatus::InOtherCollection {
                item_id: "item-1".to_string(),
                collection_id: "c1".to_string(),
            }
        );
    }

    #[test]
    fn classify_ignores_different_versions_outside_target() {
        let candidate = avail_pdf("doc.pdf", Some("aa"), None, Some(1), Some(2));
        // Different version elsewhere is not a duplicate anywhere.
        let lookup = MemLookup(vec![record(
            "item-9",
            "other",
            Some("bb"),
            None,
            Some(3),
            Some(4),
        )]);
        assert_eq!(
            classify(&candidate, "ident", &existing("c1"), &lookup),
            DuplicateStatus::New
        );
        assert_eq!(
            classify(&candidate, "ident", &new_collection(), &lookup),
            DuplicateStatus::New
        );
    }

    #[test]
    fn classify_precedence_already_over_new_version_over_other() {
        let candidate = avail_pdf("doc.pdf", Some("aa"), None, Some(1), Some(2));
        let target_match = record("item-a", "c1", Some("aa"), None, Some(1), Some(2));
        let target_other = record("item-b", "c1", Some("bb"), None, Some(9), Some(9));
        let other_match = record("item-c", "other", Some("aa"), None, Some(1), Some(2));

        // All three present: AlreadyInTarget wins.
        let lookup = MemLookup(vec![
            other_match.clone(),
            target_other.clone(),
            target_match.clone(),
        ]);
        assert_eq!(
            classify(&candidate, "ident", &existing("c1"), &lookup),
            DuplicateStatus::AlreadyInTarget {
                item_id: "item-a".to_string()
            }
        );

        // Target mismatch + other match: NewVersion wins over other.
        let lookup = MemLookup(vec![other_match.clone(), target_other.clone()]);
        assert_eq!(
            classify(&candidate, "ident", &existing("c1"), &lookup),
            DuplicateStatus::NewVersion {
                previous_item_id: "item-b".to_string()
            }
        );

        // Only the other match: InOtherCollection.
        let lookup = MemLookup(vec![other_match.clone()]);
        assert_eq!(
            classify(&candidate, "ident", &existing("c1"), &lookup),
            DuplicateStatus::InOtherCollection {
                item_id: "item-c".to_string(),
                collection_id: "other".to_string(),
            }
        );
    }

    #[test]
    fn classify_deterministic_across_record_orders() {
        let candidate = avail_pdf("doc.pdf", Some("aa"), None, Some(1), Some(2));
        let base = vec![
            record("item-b", "c1", Some("aa"), None, Some(1), Some(2)),
            record("item-a", "c1", Some("aa"), None, Some(1), Some(2)),
            record("item-c", "other", Some("aa"), None, Some(1), Some(2)),
        ];
        let first = classify(
            &candidate,
            "ident",
            &existing("c1"),
            &MemLookup(base.clone()),
        );
        // Smallest item_id among target matches wins regardless of input order.
        assert_eq!(
            first,
            DuplicateStatus::AlreadyInTarget {
                item_id: "item-a".to_string()
            }
        );
        let reversed: Vec<_> = base.into_iter().rev().collect();
        let second = classify(&candidate, "ident", &existing("c1"), &MemLookup(reversed));
        assert_eq!(first, second);

        // Same for the other-collection pile under a new collection.
        let others = vec![
            record("item-z", "cz", Some("aa"), None, Some(1), Some(2)),
            record("item-m", "cm", Some("aa"), None, Some(1), Some(2)),
        ];
        let a = classify(
            &candidate,
            "ident",
            &new_collection(),
            &MemLookup(others.clone()),
        );
        let b = classify(
            &candidate,
            "ident",
            &new_collection(),
            &MemLookup(others.into_iter().rev().collect()),
        );
        assert_eq!(a, b);
        assert_eq!(
            a,
            DuplicateStatus::InOtherCollection {
                item_id: "item-m".to_string(),
                collection_id: "cm".to_string(),
            }
        );
    }

    #[test]
    fn classify_matches_identifier_and_file_name_key() {
        let candidate = avail_pdf("wanted.pdf", Some("aa"), None, Some(1), Some(2));
        let lookup = MemLookup(vec![
            record_named(
                "item-x",
                "c1",
                "ident",
                "other.pdf",
                Some("aa"),
                None,
                Some(1),
                Some(2),
            ),
            record_named(
                "item-y",
                "c1",
                "other-ident",
                "wanted.pdf",
                Some("aa"),
                None,
                Some(1),
                Some(2),
            ),
        ]);
        // Same hash but different key: no match.
        assert_eq!(
            classify(&candidate, "ident", &existing("c1"), &lookup),
            DuplicateStatus::New
        );
    }

    // ---- summarize ----

    #[test]
    fn summarize_mixed_files() {
        let target = existing("c1");
        let lookup = MemLookup(vec![
            record_named(
                "item-a",
                "c1",
                "i2",
                "b.pdf",
                Some("bb"),
                None,
                Some(200),
                Some(2),
            ),
            record_named(
                "item-n",
                "other",
                "i4",
                "d.pdf",
                Some("dd"),
                None,
                Some(400),
                Some(4),
            ),
            record_named(
                "item-v",
                "c1",
                "i5",
                "e.pdf",
                Some("ee"),
                None,
                Some(1),
                Some(1),
            ),
        ]);
        let files: Vec<(String, PdfFile)> = vec![
            // New, known size.
            (
                "i1".to_string(),
                avail_pdf("a.pdf", Some("aa"), None, Some(100), Some(1)),
            ),
            // Already in target.
            (
                "i2".to_string(),
                avail_pdf("b.pdf", Some("bb"), None, Some(200), Some(2)),
            ),
            // Restricted: encrypted + private; sizes must not count.
            (
                "i3".to_string(),
                restricted_pdf("c.pdf", RestrictedReason::Encrypted),
            ),
            (
                "i3b".to_string(),
                restricted_pdf("c2.pdf", RestrictedReason::Private),
            ),
            // In other collection.
            (
                "i4".to_string(),
                avail_pdf("d.pdf", Some("dd"), None, Some(400), Some(4)),
            ),
            // New version in target.
            (
                "i5".to_string(),
                avail_pdf("e.pdf", Some("aa11"), None, Some(500), Some(5)),
            ),
            // Available but unknown size, no record -> New.
            (
                "i6".to_string(),
                avail_pdf("f.pdf", Some("ff"), None, None, None),
            ),
        ];
        let summary = summarize(&files, &target, &lookup);
        assert_eq!(summary.total_files, 7);
        assert_eq!(summary.available_files, 5);
        assert_eq!(summary.restricted_files, 2);
        // Known sizes of available files only: 100+200+400+500.
        assert_eq!(summary.known_bytes, 1200);
        assert_eq!(summary.unknown_size_files, 1);
        assert_eq!(summary.new_files, 2);
        assert_eq!(summary.already_in_target, 1);
        assert_eq!(summary.in_other_collection, 1);
        assert_eq!(summary.new_version, 1);
        assert_eq!(summary.entries.len(), 7);
        // Restricted rows carry no duplicate status.
        assert_eq!(summary.entries[2].duplicate, None);
        assert_eq!(summary.entries[3].duplicate, None);
        // Spot-check classified rows.
        assert_eq!(summary.entries[0].duplicate, Some(DuplicateStatus::New));
        assert_eq!(
            summary.entries[1].duplicate,
            Some(DuplicateStatus::AlreadyInTarget {
                item_id: "item-a".to_string()
            })
        );
        assert_eq!(summary.entries[1].availability, Availability::Available);
        assert_eq!(summary.entries[1].size, Some(200));
        // Order preserved.
        let names: Vec<&str> = summary
            .entries
            .iter()
            .map(|e| e.file_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["a.pdf", "b.pdf", "c.pdf", "c2.pdf", "d.pdf", "e.pdf", "f.pdf"]
        );
    }

    #[test]
    fn summarize_byte_totals_saturate() {
        let lookup = MemLookup(vec![]);
        let files = vec![
            (
                "i1".to_string(),
                avail_pdf("a.pdf", Some("a1"), None, Some(u64::MAX), Some(1)),
            ),
            (
                "i2".to_string(),
                avail_pdf("b.pdf", Some("b2"), None, Some(u64::MAX), Some(2)),
            ),
            (
                "i3".to_string(),
                restricted_pdf("c.pdf", RestrictedReason::Private),
            ),
        ];
        let summary = summarize(&files, &new_collection(), &lookup);
        assert_eq!(summary.known_bytes, u64::MAX);
        assert_eq!(summary.total_files, 3);
        assert_eq!(summary.new_files, 2);
        // Doubling a saturated total still saturates.
        assert_eq!(summary.estimated_sync_bytes(), u64::MAX);
    }

    #[test]
    fn sync_estimates_split_new_and_other_piles() {
        let target = existing("c1");
        let lookup = MemLookup(vec![
            record_named(
                "item-a",
                "c1",
                "i2",
                "b.pdf",
                Some("bb"),
                None,
                Some(200),
                Some(2),
            ),
            record_named(
                "item-n",
                "other",
                "i3",
                "c.pdf",
                Some("cc"),
                None,
                Some(300),
                Some(3),
            ),
        ]);
        let files = vec![
            (
                "i1".to_string(),
                avail_pdf("a.pdf", Some("aa"), None, Some(100), Some(1)),
            ),
            (
                "i2".to_string(),
                avail_pdf("b.pdf", Some("bb"), None, Some(200), Some(2)),
            ),
            (
                "i3".to_string(),
                avail_pdf("c.pdf", Some("cc"), None, Some(300), Some(3)),
            ),
            (
                "i4".to_string(),
                avail_pdf("d.pdf", Some("dd"), None, None, None),
            ),
            (
                "i5".to_string(),
                restricted_pdf("e.pdf", RestrictedReason::Encrypted),
            ),
        ];
        let summary = summarize(&files, &target, &lookup);
        // New (100 + unknown 0) doubled; already-in-target excluded.
        assert_eq!(summary.estimated_sync_bytes(), 200);
        // Other-collection pile doubled separately.
        assert_eq!(summary.in_other_collection_bytes(), 600);

        // A NewVersion file counts toward the main estimate.
        let lookup_v = MemLookup(vec![record_named(
            "item-v",
            "c1",
            "i1",
            "a.pdf",
            Some("ee"),
            None,
            Some(1),
            Some(1),
        )]);
        let summary_v = summarize(&files[..1], &target, &lookup_v);
        assert_eq!(
            summary_v.entries[0].duplicate,
            Some(DuplicateStatus::NewVersion {
                previous_item_id: "item-v".to_string()
            })
        );
        assert_eq!(summary_v.estimated_sync_bytes(), 200);
        assert_eq!(summary_v.in_other_collection_bytes(), 0);
    }

    #[test]
    fn sync_estimates_saturate_on_doubling() {
        let lookup = MemLookup(vec![]);
        let files = vec![(
            "i1".to_string(),
            avail_pdf("a.pdf", Some("aa"), None, Some(u64::MAX), Some(1)),
        )];
        let summary = summarize(&files, &new_collection(), &lookup);
        assert_eq!(summary.estimated_sync_bytes(), u64::MAX);
    }
}
