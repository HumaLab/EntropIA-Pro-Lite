use rusqlite::{params, OptionalExtension};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

const IMAGE_MARKER_PREFIX: &str = "OCRC_IMAGE_REFERENCE";

#[derive(Clone)]
struct ProtectedImage {
    line_index: usize,
    line: String,
    identity: String,
    marker: String,
    preceding_anchor: Option<String>,
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn image_identity(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.starts_with("![") && trimmed.ends_with(')') {
        let source_start = trimmed.find("](")? + 2;
        let source = trimmed.get(source_start..trimmed.len() - 1)?.trim();
        return (!source.is_empty()).then(|| format!("markdown:{source}"));
    }
    if trimmed.starts_with("<img") && trimmed.ends_with('>') {
        let src_start = trimmed.find("src=")? + 4;
        let quote = trimmed.as_bytes().get(src_start).copied()? as char;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let value_start = src_start + 1;
        let value_end = trimmed.get(value_start..)?.find(quote)? + value_start;
        let source = trimmed.get(value_start..value_end)?;
        return (!source.is_empty()).then(|| format!("html:{source}"));
    }
    None
}

fn is_standalone_image(line: &str) -> bool {
    image_identity(line).is_some()
}

fn protected_images(source: &str) -> Vec<ProtectedImage> {
    let lines: Vec<&str> = source.split('\n').collect();
    let mut images = Vec::new();
    let mut used_markers = HashSet::new();

    for (line_index, line) in lines.iter().enumerate() {
        if !is_standalone_image(line) {
            continue;
        }

        let mut suffix = 0usize;
        let marker = loop {
            let suffix_text = if suffix == 0 {
                String::new()
            } else {
                format!("_{suffix}")
            };
            let candidate = format!(
                "<!--{IMAGE_MARKER_PREFIX}_{:04}{suffix_text}-->",
                images.len() + 1
            );
            if !source.contains(&candidate) && used_markers.insert(candidate.clone()) {
                break candidate;
            }
            suffix += 1;
        };

        let preceding_anchor = lines[..line_index]
            .iter()
            .rev()
            .find(|candidate| !candidate.trim().is_empty() && !is_standalone_image(candidate))
            .map(|candidate| (*candidate).to_string());

        images.push(ProtectedImage {
            line_index,
            line: (*line).to_string(),
            identity: image_identity(line).expect("standalone image identity"),
            marker,
            preceding_anchor,
        });
    }

    images
}

pub(crate) fn protect_image_references(source: &str) -> String {
    let images = protected_images(source);
    if images.is_empty() {
        return source.to_string();
    }

    let by_line: std::collections::HashMap<usize, &ProtectedImage> = images
        .iter()
        .map(|image| (image.line_index, image))
        .collect();
    source
        .split('\n')
        .enumerate()
        .map(|(index, line)| {
            by_line
                .get(&index)
                .map_or_else(|| line.to_string(), |image| image.marker.clone())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn insert_missing_image(
    lines: &mut Vec<String>,
    image: &ProtectedImage,
    previous_image_line: Option<&str>,
) {
    let anchor_index = previous_image_line
        .and_then(|previous| lines.iter().rposition(|line| line == previous))
        .or_else(|| {
            image
                .preceding_anchor
                .as_ref()
                .and_then(|anchor| lines.iter().rposition(|line| line == anchor))
        });

    let mut insert_at = anchor_index
        .map(|index| index + 1)
        .unwrap_or_else(|| image.line_index.min(lines.len()));
    while insert_at < lines.len() && lines[insert_at].trim().is_empty() {
        insert_at += 1;
    }

    let needs_blank_before = insert_at > 0 && !lines[insert_at - 1].trim().is_empty();
    let needs_blank_after = insert_at < lines.len() && !lines[insert_at].trim().is_empty();
    let mut block = Vec::with_capacity(3);
    if needs_blank_before {
        block.push(String::new());
    }
    block.push(image.line.clone());
    if needs_blank_after {
        block.push(String::new());
    }
    lines.splice(insert_at..insert_at, block);
}

fn restore_image_references(original: &str, corrected: &str) -> String {
    let images = protected_images(original);
    if images.is_empty() {
        return corrected.to_string();
    }

    let original_identities: HashSet<&str> =
        images.iter().map(|image| image.identity.as_str()).collect();
    let mut lines: Vec<String> = corrected
        .split('\n')
        .filter(|line| {
            image_identity(line)
                .as_deref()
                .is_none_or(|identity| !original_identities.contains(identity))
        })
        .map(str::to_string)
        .collect();
    let mut restored_by_marker = vec![false; images.len()];

    for (image_index, image) in images.iter().enumerate() {
        let mut first_marker_seen = false;
        for line in &mut lines {
            let Some(marker_index) = line.find(&image.marker) else {
                continue;
            };
            if first_marker_seen {
                *line = line.replace(&image.marker, "");
                continue;
            }

            let after_marker = marker_index + image.marker.len();
            let remainder = line[after_marker..].replace(&image.marker, "");
            *line = format!("{}{}{}", &line[..marker_index], image.line, remainder);
            first_marker_seen = true;
            restored_by_marker[image_index] = true;
        }
    }

    for (image_index, image) in images.iter().enumerate() {
        if restored_by_marker[image_index] {
            continue;
        }
        let previous_image_line = image_index
            .checked_sub(1)
            .map(|previous_index| images[previous_index].line.as_str());
        insert_missing_image(&mut lines, image, previous_image_line);
    }

    lines.join("\n")
}

pub(crate) fn ensure_schema(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ocr_correction_backups (
            asset_id TEXT PRIMARY KEY,
            original_text_content TEXT NOT NULL,
            created_at INTEGER NOT NULL
         );",
    )
    .map_err(|error| format!("Failed to ensure OCR correction backup schema: {error}"))
}

pub(crate) fn commit_asset_correction(
    conn: &rusqlite::Connection,
    asset_id: &str,
    model_output: &str,
) -> Result<String, String> {
    if model_output.trim().is_empty() {
        return Err("OCR correction model output is empty or whitespace-only".to_string());
    }

    ensure_schema(conn)?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("Failed to begin OCR correction transaction: {error}"))?;

    let existing_backup: Option<String> = tx
        .query_row(
            "SELECT original_text_content FROM ocr_correction_backups WHERE asset_id = ?1",
            [asset_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed to read OCR correction backup: {error}"))?;

    let current_extraction: Option<String> = tx
        .query_row(
            "SELECT text_content FROM extractions
             WHERE asset_id = ?1
             ORDER BY created_at DESC
             LIMIT 1",
            [asset_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed to read OCR extraction: {error}"))?;
    let current_extraction = current_extraction
        .ok_or_else(|| format!("No extraction available for OCR correction on asset {asset_id}"))?;
    let original = existing_backup.as_deref().unwrap_or(&current_extraction);
    let corrected = restore_image_references(original, model_output);
    let now = now_millis();

    tx.execute(
        "INSERT OR IGNORE INTO ocr_correction_backups(
            asset_id, original_text_content, created_at
         ) VALUES (?1, ?2, ?3)",
        params![asset_id, original, now],
    )
    .map_err(|error| format!("Failed to save original OCR backup: {error}"))?;

    let updated = tx
        .execute(
            "UPDATE extractions SET text_content = ?1 WHERE asset_id = ?2",
            params![corrected, asset_id],
        )
        .map_err(|error| format!("Failed to replace OCR extraction: {error}"))?;
    if updated == 0 {
        return Err(format!(
            "No extraction available for OCR correction on asset {asset_id}"
        ));
    }

    let result_id = format!("llr-asset-{asset_id}-correct_ocr");
    tx.execute(
        "INSERT INTO llm_results(id, target_id, target_type, job_type, result, created_at)
         VALUES (?1, ?2, 'asset', 'correct_ocr', ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
            target_id = excluded.target_id,
            target_type = excluded.target_type,
            job_type = excluded.job_type,
            result = excluded.result,
            created_at = excluded.created_at",
        params![result_id, asset_id, corrected, now],
    )
    .map_err(|error| format!("Failed to persist OCR correction result: {error}"))?;

    tx.commit()
        .map_err(|error| format!("Failed to commit OCR correction transaction: {error}"))?;
    Ok(corrected)
}

pub(crate) fn can_restore_original(
    conn: &rusqlite::Connection,
    asset_id: &str,
) -> Result<bool, String> {
    ensure_schema(conn)?;
    conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM ocr_correction_backups WHERE asset_id = ?1
         )",
        [asset_id],
        |row| row.get(0),
    )
    .map_err(|error| format!("Failed to inspect OCR correction backup: {error}"))
}

pub(crate) fn restore_original(
    conn: &rusqlite::Connection,
    asset_id: &str,
) -> Result<String, String> {
    ensure_schema(conn)?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| format!("Failed to begin OCR restore transaction: {error}"))?;
    let original: Option<String> = tx
        .query_row(
            "SELECT original_text_content FROM ocr_correction_backups WHERE asset_id = ?1",
            [asset_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Failed to read original OCR backup: {error}"))?;
    let original =
        original.ok_or_else(|| format!("No original OCR backup exists for asset {asset_id}"))?;

    let updated = tx
        .execute(
            "UPDATE extractions SET text_content = ?1 WHERE asset_id = ?2",
            params![original, asset_id],
        )
        .map_err(|error| format!("Failed to restore original OCR extraction: {error}"))?;
    if updated == 0 {
        return Err(format!(
            "No extraction available while restoring OCR asset {asset_id}"
        ));
    }

    tx.execute(
        "DELETE FROM llm_results
         WHERE target_id = ?1
           AND (target_type = 'asset' OR target_type = 'unknown')
           AND job_type = 'correct_ocr'",
        [asset_id],
    )
    .map_err(|error| format!("Failed to delete corrected OCR result: {error}"))?;
    tx.execute(
        "DELETE FROM ocr_correction_backups WHERE asset_id = ?1",
        [asset_id],
    )
    .map_err(|error| format!("Failed to delete restored OCR backup: {error}"))?;
    tx.commit()
        .map_err(|error| format!("Failed to commit OCR restore transaction: {error}"))?;
    Ok(original)
}

pub(crate) fn clear_asset_state(conn: &rusqlite::Connection, asset_id: &str) -> Result<(), String> {
    ensure_schema(conn)?;
    conn.execute(
        "DELETE FROM llm_results
         WHERE target_id = ?1
           AND (target_type = 'asset' OR target_type = 'unknown')
           AND job_type = 'correct_ocr'",
        [asset_id],
    )
    .map_err(|error| format!("Failed to clear corrected OCR result: {error}"))?;
    conn.execute(
        "DELETE FROM ocr_correction_backups WHERE asset_id = ?1",
        [asset_id],
    )
    .map_err(|error| format!("Failed to clear original OCR backup: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    const MARKDOWN_IMAGE: &str = "![OCR region](page=1&bbox=10,20,30,40)";
    const HTML_IMAGE: &str = r#"<img src="data:image/png;base64,AAAA" alt="scan">"#;

    fn correction_db(original: &str) -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory correction db");
        conn.execute_batch(
            "CREATE TABLE extractions (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL UNIQUE,
                text_content TEXT NOT NULL,
                method TEXT NOT NULL,
                confidence REAL,
                created_at INTEGER NOT NULL
             );
             CREATE TABLE llm_results (
                id TEXT PRIMARY KEY,
                target_id TEXT NOT NULL,
                target_type TEXT NOT NULL,
                job_type TEXT NOT NULL,
                result TEXT NOT NULL,
                created_at INTEGER NOT NULL
             );",
        )
        .expect("correction schema");
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, method, created_at)
             VALUES ('ext-asset-1', 'asset-1', ?1, 'ocr', 1)",
            [original],
        )
        .expect("original extraction");
        conn
    }

    fn extraction_text(conn: &Connection) -> String {
        conn.query_row(
            "SELECT text_content FROM extractions WHERE asset_id = 'asset-1'",
            [],
            |row| row.get(0),
        )
        .expect("extraction text")
    }

    #[test]
    fn protects_and_restores_standalone_markdown_and_html_images_exactly_once() {
        let original = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\nTexto\n\n{HTML_IMAGE}\n\nFinal");

        let protected = protect_image_references(&original);
        assert!(!protected.contains(MARKDOWN_IMAGE));
        assert!(!protected.contains(HTML_IMAGE));
        assert_eq!(protected.matches("OCRC_IMAGE_REFERENCE").count(), 2);

        let restored = restore_image_references(&original, &protected);
        assert_eq!(restored.matches(MARKDOWN_IMAGE).count(), 1);
        assert_eq!(restored.matches(HTML_IMAGE).count(), 1);
        assert!(restored.find(MARKDOWN_IMAGE).unwrap() < restored.find(HTML_IMAGE).unwrap());
    }

    #[test]
    fn missing_markers_fall_back_after_the_nearest_preserved_anchor_in_original_order() {
        let second = "![Second](page=1&bbox=50,60,70,80)";
        let original = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\n{second}\n\nTexto original");
        let corrected_without_markers = "# Documento\n\nTexto corregido";

        let restored = restore_image_references(&original, corrected_without_markers);

        let heading = restored.find("# Documento").unwrap();
        let first = restored.find(MARKDOWN_IMAGE).unwrap();
        let second = restored.find(second).unwrap();
        let text = restored.find("Texto corregido").unwrap();
        assert!(heading < first && first < second && second < text);
    }

    #[test]
    fn existing_exact_images_are_not_duplicated_when_the_model_drops_the_marker() {
        let original = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\nTexto");
        let corrected = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\nTexto corregido");

        let restored = restore_image_references(&original, &corrected);

        assert_eq!(restored.matches(MARKDOWN_IMAGE).count(), 1);
    }

    #[test]
    fn an_existing_image_source_is_not_duplicated_when_alt_text_changes() {
        let original = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\nTexto");
        let corrected =
            "# Documento\n\n![Descripción cambiada](page=1&bbox=10,20,30,40)\n\nTexto corregido";

        let restored = restore_image_references(&original, corrected);

        assert_eq!(restored.matches("page=1&bbox=10,20,30,40").count(), 1);
    }

    #[test]
    fn duplicate_original_sources_are_restored_by_occurrence() {
        let original =
            format!("# Documento\n\n{MARKDOWN_IMAGE}\n\n{MARKDOWN_IMAGE}\n\nTexto original");
        let protected = protect_image_references(&original);
        let corrected = protected.replacen(
            "<!--OCRC_IMAGE_REFERENCE_0001-->",
            "![Descripción cambiada](page=1&bbox=10,20,30,40)",
            1,
        );

        let restored = restore_image_references(&original, &corrected);

        assert_eq!(restored.matches("page=1&bbox=10,20,30,40").count(), 2);
    }

    #[test]
    fn a_missing_sibling_is_restored_after_an_already_retained_image() {
        let second = "![Second](page=1&bbox=50,60,70,80)";
        let original = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\n{second}\n\nTexto original");
        let corrected = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\nTexto corregido");

        let restored = restore_image_references(&original, &corrected);

        let first_index = restored.find(MARKDOWN_IMAGE).unwrap();
        let second_index = restored.find(second).unwrap();
        assert!(first_index < second_index);
    }

    #[test]
    fn commit_is_atomic_and_restore_is_one_level() {
        let original = format!("# Documento\n\n{MARKDOWN_IMAGE}\n\nTexto original");
        let conn = correction_db(&original);
        let protected = protect_image_references(&original);
        let model_output = protected.replace("Texto original", "Texto corregido");

        let corrected =
            commit_asset_correction(&conn, "asset-1", &model_output).expect("commit correction");

        assert_eq!(corrected.matches(MARKDOWN_IMAGE).count(), 1);
        assert_eq!(extraction_text(&conn), corrected);
        assert!(can_restore_original(&conn, "asset-1").unwrap());
        let stored_original: String = conn
            .query_row(
                "SELECT original_text_content FROM ocr_correction_backups WHERE asset_id = 'asset-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_original, original);
        let stored_result: String = conn
            .query_row(
                "SELECT result FROM llm_results WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_result, corrected);

        let restored = restore_original(&conn, "asset-1").expect("restore original");
        assert_eq!(restored, original);
        assert_eq!(extraction_text(&conn), original);
        assert!(!can_restore_original(&conn, "asset-1").unwrap());
        assert!(restore_original(&conn, "asset-1").is_err());
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM llm_results WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn ocr_correction_contract_rejects_whitespace_before_transaction_and_preserves_fresh_state() {
        let original = "Texto original";
        let conn = correction_db(original);
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();

        let error = commit_asset_correction(&conn, "asset-1", " \n\t ").unwrap_err();
        let normalized_error = error.to_ascii_lowercase();
        assert!(
            normalized_error.contains("empty") || normalized_error.contains("whitespace"),
            "unexpected whitespace-output error: {error}"
        );

        conn.execute_batch("ROLLBACK").unwrap();
        ensure_schema(&conn).unwrap();
        assert_eq!(extraction_text(&conn), original);
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM ocr_correction_backups WHERE asset_id = 'asset-1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM llm_results
                 WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn ocr_correction_contract_whitespace_preserves_existing_backup_extraction_and_result() {
        let conn = correction_db("Texto original");
        let corrected =
            commit_asset_correction(&conn, "asset-1", "Texto corregido").unwrap();
        let extraction_before = extraction_text(&conn);
        let backup_before: (String, i64) = conn
            .query_row(
                "SELECT original_text_content, created_at
                 FROM ocr_correction_backups WHERE asset_id = 'asset-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let result_before: (String, i64) = conn
            .query_row(
                "SELECT result, created_at FROM llm_results
                 WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        let error = commit_asset_correction(&conn, "asset-1", "\r\n  ").unwrap_err();
        let normalized_error = error.to_ascii_lowercase();
        assert!(
            normalized_error.contains("empty") || normalized_error.contains("whitespace"),
            "unexpected whitespace-output error: {error}"
        );

        assert_eq!(extraction_text(&conn), extraction_before);
        assert_eq!(extraction_before, corrected);
        assert_eq!(
            conn.query_row(
                "SELECT original_text_content, created_at
                 FROM ocr_correction_backups WHERE asset_id = 'asset-1'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap(),
            backup_before
        );
        assert_eq!(
            conn.query_row(
                "SELECT result, created_at FROM llm_results
                 WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap(),
            result_before
        );
    }

    #[test]
    fn ocr_correction_contract_restore_rolls_back_all_state_when_backup_delete_fails() {
        let conn = correction_db("Texto original");
        let corrected =
            commit_asset_correction(&conn, "asset-1", "Texto corregido").unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_ocr_backup_delete
             BEFORE DELETE ON ocr_correction_backups
             BEGIN
               SELECT RAISE(ABORT, 'forced backup delete failure');
             END;",
        )
        .unwrap();

        let error = restore_original(&conn, "asset-1").unwrap_err();

        assert!(error.contains("backup"));
        assert_eq!(extraction_text(&conn), corrected);
        assert!(can_restore_original(&conn, "asset-1").unwrap());
        assert_eq!(
            conn.query_row(
                "SELECT result FROM llm_results
                 WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            corrected
        );
    }

    #[test]
    fn second_correction_keeps_the_first_original_backup() {
        let original = "# Documento\n\nTexto original";
        let conn = correction_db(original);

        commit_asset_correction(&conn, "asset-1", "Primera corrección").unwrap();
        commit_asset_correction(&conn, "asset-1", "Segunda corrección").unwrap();

        let restored = restore_original(&conn, "asset-1").unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn missing_extraction_or_backup_never_creates_partial_version_state() {
        let conn = correction_db("Texto original");

        let error = commit_asset_correction(&conn, "missing", "corregido").unwrap_err();
        assert!(error.contains("extraction"));
        assert!(!can_restore_original(&conn, "missing").unwrap());
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM llm_results WHERE target_id = 'missing'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
        assert!(restore_original(&conn, "asset-1").is_err());
    }

    #[test]
    fn clearing_state_removes_backup_and_correction_without_touching_fresh_extraction() {
        let conn = correction_db("Texto original");
        commit_asset_correction(&conn, "asset-1", "Texto corregido").unwrap();
        conn.execute(
            "UPDATE extractions SET text_content = 'OCR fresco' WHERE asset_id = 'asset-1'",
            [],
        )
        .unwrap();

        clear_asset_state(&conn, "asset-1").unwrap();

        assert_eq!(extraction_text(&conn), "OCR fresco");
        assert!(!can_restore_original(&conn, "asset-1").unwrap());
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM llm_results WHERE target_id = 'asset-1' AND job_type = 'correct_ocr'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
    }
}
