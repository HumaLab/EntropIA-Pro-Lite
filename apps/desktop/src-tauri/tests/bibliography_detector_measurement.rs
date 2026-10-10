//! B2: bibliography garble detector measurement on a READ-ONLY copy of a
//! real archive database (plan-texto-nativo-parte-b 2.2, "Validación antes
//! de mergear").
//!
//! Both tests are `#[ignore]` and env-driven; they never write to the
//! database they measure — open it read-only, and run them against a COPY
//! made with the SQLite online backup API (`.backup`), never the live one:
//!
//! ```text
//! ENTROPIA_MEASURE_DB=G:/EntropIA-Stack/agent-scratch/prueba-sync-copy.sqlite \
//!   cargo test --test bibliography_detector_measurement -- --ignored --nocapture
//! ```
//!
//! For every PDF attachment with stored `bibliographic_page_texts` rows the
//! measurement resolves the file with the app's own resolver, reads the
//! stored-`rich` pages with the part-A PDFium reader, and reports how the
//! detector splits them (rule 1 vs rule 2), plus how many STORED page texts
//! it flags — the size of the reprocess candidate list. Full page texts
//! never leave the process; samples carry at most 200 characters.

use std::collections::HashSet;

use entropia_desktop_lib::bibliography::attachment::{
    attachment_ref_for, resolve_attachment_file, AttachmentResolution,
};
use entropia_desktop_lib::bibliography::processing::{
    garbled_bibliography_flags, pdfium_page_texts_raw, BIBLIOGRAPHY_DETECTOR_VERSION,
    ZOTERO_DATA_DIR_SETTING_KEY,
};
use entropia_desktop_lib::get_setting;

/// The read-only copy to measure; the tests skip without it.
fn measure_db_path() -> Option<std::path::PathBuf> {
    match std::env::var("ENTROPIA_MEASURE_DB") {
        Ok(path) if !path.trim().is_empty() => Some(std::path::PathBuf::from(path)),
        _ => None,
    }
}

fn open_read_only_copy() -> Option<rusqlite::Connection> {
    let path = measure_db_path()?;
    match rusqlite::Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(conn) => Some(conn),
        Err(error) => panic!(
            "could not open the read-only copy {}: {error}",
            path.display()
        ),
    }
}

fn is_pdf_attachment(content_type: Option<&str>, filename: Option<&str>) -> bool {
    content_type == Some("application/pdf")
        || filename.is_some_and(|name| name.to_ascii_lowercase().ends_with(".pdf"))
}

/// One stored page row.
struct StoredPage {
    number: i64,
    quality: String,
    text: String,
}

fn stored_pages(conn: &rusqlite::Connection, attachment_id: &str) -> Vec<StoredPage> {
    let mut stmt = conn
        .prepare(
            "SELECT page_number, quality, text_content
             FROM bibliographic_page_texts WHERE attachment_id = ?1 ORDER BY page_number",
        )
        .expect("prepare the page-row query");
    let rows = stmt
        .query_map([attachment_id], |row| {
            Ok(StoredPage {
                number: row.get(0)?,
                quality: row.get(1)?,
                text: row.get(2)?,
            })
        })
        .expect("query the page rows");
    rows.filter_map(Result::ok).collect()
}

/// Deterministic xorshift so the sample in the report is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// One flagged PDFium page for the sample in the report.
struct FlaggedPage {
    attachment_id: String,
    page: i64,
    rule: String,
    preview: String,
}

#[test]
#[ignore = "measurement on a read-only database copy; run with ENTROPIA_MEASURE_DB set"]
fn measure_bibliography_detector_on_db_copy() {
    let Some(conn) = open_read_only_copy() else {
        eprintln!("ENTROPIA_MEASURE_DB unset; skipping");
        return;
    };
    let data_dir = get_setting(&conn, ZOTERO_DATA_DIR_SETTING_KEY);
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT p.attachment_id, a.content_type, a.filename
             FROM bibliographic_page_texts p
             JOIN zotero_attachments a ON a.id = p.attachment_id
             ORDER BY p.attachment_id",
        )
        .expect("prepare the attachment query");
    let attachments: Vec<(String, Option<String>, Option<String>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query the attachments")
        .filter_map(Result::ok)
        .collect();

    let mut rng = Rng(0x5DEECE66D);
    let mut attachments_scanned = 0usize;
    let mut attachments_unresolved = 0usize;
    let mut pages_stored = 0usize;
    let mut rich_pages_with_pdfium_text = 0usize;
    let mut pdfium_rule1 = 0usize;
    let mut pdfium_rule2 = 0usize;
    let mut pdfium_rule3 = 0usize;
    let mut pdfium_flagged = 0usize;
    let mut stored_flagged = 0usize;
    let mut stored_rule1 = 0usize;
    let mut stored_rule2 = 0usize;
    let mut stored_rule3 = 0usize;
    let mut stored_flagged_rich = 0usize;
    let mut stored_flagged_attachments: HashSet<String> = HashSet::new();
    let mut pdfium_flagged_attachments: HashSet<String> = HashSet::new();
    let mut sample: Vec<FlaggedPage> = Vec::new();
    let mut sample_seen = 0usize;
    const SAMPLE_SIZE: usize = 30;
    let mut all_flagged: Vec<FlaggedPage> = Vec::new();
    let mut pdfium_flagged_per_attachment: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    for (attachment_id, content_type, filename) in &attachments {
        if !is_pdf_attachment(content_type.as_deref(), filename.as_deref()) {
            continue;
        }
        let pages = stored_pages(&conn, attachment_id);
        if pages.is_empty() {
            continue;
        }
        attachments_scanned += 1;
        pages_stored += pages.len();
        eprintln!(
            "[measure] #{attachments_scanned} {attachment_id}: {} stored pages",
            pages.len()
        );

        // The detector over the STORED page texts sizes the reprocess list.
        for page in &pages {
            let flags = garbled_bibliography_flags(&page.text);
            if flags.glued_words {
                stored_rule1 += 1;
            }
            if flags.old_ocr_noise {
                stored_rule2 += 1;
            }
            if flags.punctuation_soup {
                stored_rule3 += 1;
            }
            if flags.glued_words || flags.old_ocr_noise || flags.punctuation_soup {
                stored_flagged += 1;
                if page.quality == "rich" {
                    stored_flagged_rich += 1;
                }
                stored_flagged_attachments.insert(attachment_id.clone());
            }
        }

        // The detector over the PDFium text of every stored-`rich` page.
        let rich_numbers: Vec<u32> = pages
            .iter()
            .filter(|page| page.quality == "rich")
            .filter_map(|page| u32::try_from(page.number).ok())
            .collect();
        if rich_numbers.is_empty() {
            continue;
        }
        let attachment = attachment_ref_for(&conn, attachment_id).expect("read attachment ref");
        let Some(attachment) = attachment else {
            attachments_unresolved += 1;
            continue;
        };
        let path = match resolve_attachment_file(&attachment, data_dir.as_deref()) {
            AttachmentResolution::File(path) => path,
            AttachmentResolution::Unavailable { .. } => {
                attachments_unresolved += 1;
                continue;
            }
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                attachments_unresolved += 1;
                continue;
            }
        };
        let texts = match pdfium_page_texts_raw(&bytes, &rich_numbers) {
            Ok(texts) => texts,
            Err(error) => {
                eprintln!("[measure] {attachment_id}: PDFium unavailable ({error})");
                attachments_unresolved += 1;
                continue;
            }
        };
        for (number, text) in texts {
            let Some(text) = text.filter(|text| !text.trim().is_empty()) else {
                continue;
            };
            rich_pages_with_pdfium_text += 1;
            let flags = garbled_bibliography_flags(&text);
            let mut rules: Vec<&str> = Vec::new();
            if flags.glued_words {
                pdfium_rule1 += 1;
                rules.push("rule 1 (glued words)");
            }
            if flags.old_ocr_noise {
                pdfium_rule2 += 1;
                rules.push("rule 2 (old OCR noise)");
            }
            if flags.punctuation_soup {
                pdfium_rule3 += 1;
                rules.push("rule 3 (punctuation soup)");
            }
            if rules.is_empty() {
                continue;
            }
            let rule = rules.join(" + ");
            pdfium_flagged += 1;
            pdfium_flagged_attachments.insert(attachment_id.clone());
            *pdfium_flagged_per_attachment
                .entry(attachment_id.clone())
                .or_insert(0) += 1;
            all_flagged.push(FlaggedPage {
                attachment_id: attachment_id.clone(),
                page: i64::from(number),
                rule: rule.clone(),
                preview: text.chars().take(200).collect(),
            });
            // Reservoir sampling keeps 30 pages of the flagged stream.
            sample_seen += 1;
            if sample.len() < SAMPLE_SIZE {
                sample.push(FlaggedPage {
                    attachment_id: attachment_id.clone(),
                    page: i64::from(number),
                    rule: rule.clone(),
                    preview: text.chars().take(200).collect(),
                });
            } else {
                let slot = (rng.next() % sample_seen as u64) as usize;
                if slot < SAMPLE_SIZE {
                    sample[slot] = FlaggedPage {
                        attachment_id: attachment_id.clone(),
                        page: i64::from(number),
                        rule: rule.clone(),
                        preview: text.chars().take(200).collect(),
                    };
                }
            }
        }
    }

    let pct = |part: usize, whole: usize| {
        if whole == 0 {
            0.0
        } else {
            100.0 * part as f64 / whole as f64
        }
    };
    println!("# Native-text bibliography detector — measurement");
    println!();
    println!("- detector version: {BIBLIOGRAPHY_DETECTOR_VERSION}");
    println!("- PDF attachments with stored page rows scanned: {attachments_scanned}");
    println!("- attachments whose file could not be read: {attachments_unresolved}");
    println!("- stored page rows seen: {pages_stored}");
    println!("- stored-`rich` pages with a PDFium read: {rich_pages_with_pdfium_text}");
    println!();
    println!("## PDFium text of stored-`rich` pages");
    println!();
    println!("- flagged by rule 1 (glued words): {pdfium_rule1}");
    println!("- flagged by rule 2 (old OCR noise): {pdfium_rule2}");
    println!("- flagged by rule 3 (punctuation soup): {pdfium_rule3}");
    println!(
        "- flagged by any rule: {pdfium_flagged} ({:.2} % of scanned)",
        pct(pdfium_flagged, rich_pages_with_pdfium_text)
    );
    println!(
        "- distinct attachments with a flagged page: {}",
        pdfium_flagged_attachments.len()
    );
    println!("- flagged PDFium pages per attachment:");
    for (attachment_id, count) in &pdfium_flagged_per_attachment {
        println!("  - `{attachment_id}`: {count}");
    }
    println!();
    println!(
        "## Flagged pages of low-flag attachments (every one, for the false-positive reading)"
    );
    println!();
    for page in &all_flagged {
        if pdfium_flagged_per_attachment
            .get(&page.attachment_id)
            .is_some_and(|count| *count < 5)
        {
            let preview = page.preview.replace('\n', " ");
            println!(
                "- `{}` p. {} — {}: {}",
                page.attachment_id, page.page, page.rule, preview
            );
        }
    }
    println!();
    println!("## STORED page texts (whole reprocess candidate list)");
    println!();
    println!("- flagged by rule 1 (glued words): {stored_rule1}");
    println!("- flagged by rule 2 (old OCR noise): {stored_rule2}");
    println!("- flagged by rule 3 (punctuation soup): {stored_rule3}");
    println!(
        "- flagged by any rule: {stored_flagged} ({:.2} % of stored rows)",
        pct(stored_flagged, pages_stored)
    );
    println!("- of those, stored quality `rich`: {stored_flagged_rich}");
    println!(
        "- distinct attachments with a flagged stored page: {}",
        stored_flagged_attachments.len()
    );
    println!();
    println!("## 30 randomly sampled flagged PDFium pages");
    println!();
    for (index, page) in sample.iter().enumerate() {
        let preview = page.preview.replace('\n', " ");
        println!(
            "{}. `{}` p. {} — {}: {}",
            index + 1,
            page.attachment_id,
            page.page,
            page.rule,
            preview
        );
    }
}

/// Fixture provenance: prints the PDFium read of the page whose STORED row
/// is the glued Abulafia 1950 p. 309 text, so the fixtures in `ocr/pdf.rs`
/// can be cut verbatim from a real extraction (both sides).
#[test]
#[ignore = "fixture extraction from the database copy; run with ENTROPIA_MEASURE_DB set"]
fn print_p309_pdfium_fixture_source() {
    let Some(conn) = open_read_only_copy() else {
        eprintln!("ENTROPIA_MEASURE_DB unset; skipping");
        return;
    };
    let data_dir = get_setting(&conn, ZOTERO_DATA_DIR_SETTING_KEY);
    let (attachment_id, page_number): (String, i64) = conn
        .query_row(
            "SELECT attachment_id, page_number FROM bibliographic_page_texts
             WHERE text_content LIKE '%ArrozElcultivodelarroz%' LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("find the glued Abulafia p. 309 stored row");
    let attachment = attachment_ref_for(&conn, &attachment_id)
        .expect("read attachment ref")
        .expect("attachment row exists");
    let AttachmentResolution::File(path) =
        resolve_attachment_file(&attachment, data_dir.as_deref())
    else {
        panic!("the fixture attachment file must resolve");
    };
    let bytes = std::fs::read(&path).expect("read the fixture PDF");
    let number = u32::try_from(page_number).expect("page number fits u32");
    let mut texts = pdfium_page_texts_raw(&bytes, &[number]).expect("PDFium read");
    let (_, text) = texts.pop().expect("one page back");
    let text = text.expect("PDFium reads the fixture page");
    println!(
        "attachment: {attachment_id} page: {page_number} chars: {}",
        text.chars().count()
    );
    println!("--- pdfium text ---");
    println!("{text}");
}
