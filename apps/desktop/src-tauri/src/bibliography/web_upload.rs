//! Web API file-upload transport (E5b-upload): the verified
//! `create → register → S3 → finalize → readback` flow against
//! `api.zotero.org`. Local connector and local `/api/*` have no file path
//! (proven 2026-09-24); this transport is the only writer of bytes, and it
//! only targets the explicitly constructed group. The API key enters at
//! construction, is never logged, and never appears in receipts.

use rusqlite::{Connection, OptionalExtension as _};

use super::ingest::{
    IngestFailure, IngestOperation, IngestOutcome, IngestReceipt, IngestTransport,
    KIND_UPLOAD_ATTACHMENT,
};

/// Hard cap for one upload (Zotero free group storage is small; anything
/// bigger fails `file_too_large` before any HTTP happens).
pub const MAX_UPLOAD_BYTES: u64 = 50 * 1024 * 1024;

/// One validated upload decision: the catalog parent plus the local file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadPlan {
    pub parent_item_id: String,
    pub parent_item_key: String,
    pub file_path: String,
    pub filename: String,
    pub content_type: String,
}

/// Parses and validates an `upload_attachment` payload:
/// `{mode, item_id, file_path, filename?}`. Missing pieces fail
/// `invalid_payload`; oversized files fail `file_too_large` — all before
/// any network.
pub fn upload_plan_from_payload(
    conn: &Connection,
    operation: &IngestOperation,
) -> Result<UploadPlan, IngestFailure> {
    let payload: serde_json::Value =
        serde_json::from_str(&operation.payload_json).map_err(|_| invalid_payload())?;
    if payload.get("mode").and_then(|value| value.as_str()) != Some("upload") {
        return Err(invalid_payload());
    }
    let parent_item_id = payload
        .get("item_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(invalid_payload)?;
    let file_path = payload
        .get("file_path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(invalid_payload)?;
    // The decision names one internal library row; resolve the catalog
    // parent and refuse anything outside it before any network.
    let row: Option<(String, String, Option<String>)> = conn
        .query_row(
            "SELECT i.item_key, i.library_id,
                    (SELECT t.item_id FROM zotero_item_tombstones t WHERE t.item_id = i.id)
             FROM bibliographic_items i WHERE i.id = ?1",
            [parent_item_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| {
            terminal(
                "sql_error",
                format!("No se pudo resolver el padre: {error}"),
            )
        })?;
    let (parent_key, parent_library, tombstone) = row.ok_or_else(|| {
        terminal(
            "unknown_item",
            "El trabajo padre ya no está en el catálogo.",
        )
    })?;
    if tombstone.is_some() {
        return Err(terminal(
            "tombstoned",
            "El trabajo padre fue revocado en Zotero.",
        ));
    }
    if parent_library != operation.library_id {
        return Err(terminal(
            "wrong_library",
            "El trabajo padre pertenece a otra biblioteca.",
        ));
    }
    let metadata = std::fs::metadata(file_path).map_err(|_| {
        terminal(
            "file_missing",
            "El archivo ya no está en disco.".to_string(),
        )
    })?;
    if metadata.len() > MAX_UPLOAD_BYTES {
        return Err(terminal(
            "file_too_large",
            format!("El archivo supera el máximo de {MAX_UPLOAD_BYTES} bytes."),
        ));
    }
    let filename = payload
        .get("filename")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            std::path::Path::new(file_path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("adjunto")
                .to_string()
        });
    Ok(UploadPlan {
        parent_item_id: parent_item_id.to_string(),
        parent_item_key: parent_key,
        file_path: file_path.to_string(),
        filename: filename.clone(),
        content_type: content_type_for_filename(&filename).to_string(),
    })
}

fn invalid_payload() -> IngestFailure {
    terminal(
        "invalid_payload",
        "La subida no trae padre y archivo válidos.",
    )
}

fn terminal(code: &str, message: impl Into<String>) -> IngestFailure {
    IngestFailure {
        terminal: true,
        code: code.to_string(),
        message: message.into(),
    }
}

/// Builds the attachment-item JSON the Web API accepts
/// (`linkMode: imported_file` is mandatory; `filesize` is rejected; md5
/// and mtime travel in the register step, never here — pre-registering
/// them makes the server answer 412 `file exists`).
pub fn attachment_item_body(
    parent_key: &str,
    filename: &str,
    content_type: &str,
) -> serde_json::Value {
    serde_json::json!([{
        "itemType": "attachment",
        "parentItem": parent_key,
        "linkMode": "imported_file",
        "filename": filename,
        "contentType": content_type,
        "title": filename,
    }])
}

/// Assembles the S3 multipart body: `prefix + file bytes + suffix`, byte
/// for byte what the register step authorized.
pub fn s3_multipart_body(prefix: &[u8], file_bytes: &[u8], suffix: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(prefix.len() + file_bytes.len() + suffix.len());
    body.extend_from_slice(prefix);
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(suffix);
    body
}

/// Infers the upload content type from the filename. PDFs and the common
/// text/image cases map explicitly; anything else rides as a byte stream
/// instead of failing the operation.
pub fn content_type_for_filename(filename: &str) -> &'static str {
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".pdf") {
        "application/pdf"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".txt") {
        "text/plain"
    } else if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html"
    } else {
        "application/octet-stream"
    }
}

/// Web API file-upload transport: the verified `create → register →
/// S3 → finalize → readback` flow against `api.zotero.org`. Only targets
/// the constructed group; the API key enters at construction and is never
/// logged. Handles `upload_attachment` only — anything else fails
/// `invalid_kind` before any network.
pub struct WebApiUploadTransport {
    base_url: String,
    group_id: String,
    api_key: String,
    client: reqwest::blocking::Client,
}

/// S3 upload authorization returned by the register step.
#[derive(Debug, Clone, PartialEq, Eq)]
struct S3Auth {
    url: String,
    content_type: String,
    prefix: Vec<u8>,
    suffix: Vec<u8>,
    upload_key: String,
}

impl WebApiUploadTransport {
    pub fn new(api_base: &str, group_id: &str, api_key: &str) -> Result<Self, IngestFailure> {
        if group_id.trim().is_empty() || api_key.trim().is_empty() {
            return Err(terminal(
                "invalid_config",
                "Falta el grupo o la clave de Zotero.",
            ));
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|error| {
                terminal(
                    "http_client",
                    format!("No se pudo preparar el cliente: {error}"),
                )
            })?;
        Ok(Self {
            base_url: api_base.trim_end_matches('/').to_string(),
            group_id: group_id.to_string(),
            api_key: api_key.to_string(),
            client,
        })
    }

    fn items_url(&self) -> String {
        format!("{}/groups/{}/items", self.base_url, self.group_id)
    }

    fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::blocking::Response, IngestFailure> {
        self.client
            .post(url)
            .header("Zotero-API-Key", &self.api_key)
            .json(body)
            .send()
            .map_err(|error| web_offline(error.to_string()))
    }

    /// Lists the remote children of one parent, for the lost-response
    /// pre-check: same filename + md5 means a previous attempt landed.
    fn remote_children(&self, parent_key: &str) -> Result<Vec<serde_json::Value>, IngestFailure> {
        let response = self
            .client
            .get(format!("{}/{}/children", self.items_url(), parent_key))
            .header("Zotero-API-Key", &self.api_key)
            .query(&[("format", "json"), ("limit", "100")])
            .send()
            .map_err(|error| web_offline(error.to_string()))?;
        classify(&response, "No se pudieron leer los adjuntos")?;
        response
            .json()
            .map_err(|error| terminal("web_rejected", format!("Respuesta ilegible: {error}")))
    }

    fn create_attachment_item(&self, plan: &UploadPlan) -> Result<String, IngestFailure> {
        let response = self.post_json(
            &self.items_url(),
            &attachment_item_body(&plan.parent_item_key, &plan.filename, &plan.content_type),
        )?;
        let (status, body) = read_json(response)?;
        if !(200..=299).contains(&status) {
            return Err(web_status(status, &body));
        }
        let key = body
            .get("successful")
            .and_then(|value| value.get("0"))
            .and_then(|value| value.get("key"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                let detail = body
                    .get("failed")
                    .and_then(|value| value.get("0"))
                    .and_then(|value| value.get("message"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("sin detalle");
                terminal(
                    "web_rejected",
                    format!("Zotero rechazó el adjunto: {detail}"),
                )
            })?;
        Ok(key.to_string())
    }

    fn register_upload(
        &self,
        key: &str,
        plan: &UploadPlan,
        md5_hex: &str,
        mtime_ms: u64,
        file_len: u64,
        file_bytes: &[u8],
    ) -> Result<S3Auth, IngestFailure> {
        let response = self
            .client
            .post(format!("{}/{}/file", self.items_url(), key))
            .header("Zotero-API-Key", &self.api_key)
            .header("Content-Type", plan.content_type.clone())
            .header("If-None-Match", "*")
            .query(&[
                ("md5", md5_hex.to_string()),
                ("filename", plan.filename.clone()),
                ("mtime", mtime_ms.to_string()),
                ("filesize", file_len.to_string()),
            ])
            .body(file_bytes.to_vec())
            .send()
            .map_err(|error| web_offline(error.to_string()))?;
        let (status, body) = read_json(response)?;
        if !(200..=299).contains(&status) {
            return Err(web_status(status, &body));
        }
        Ok(S3Auth {
            url: body
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            content_type: body
                .get("contentType")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            prefix: body
                .get("prefix")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .as_bytes()
                .to_vec(),
            suffix: body
                .get("suffix")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .as_bytes()
                .to_vec(),
            upload_key: body
                .get("uploadKey")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }

    fn post_to_s3(&self, auth: &S3Auth, file_bytes: &[u8]) -> Result<(), IngestFailure> {
        if auth.url.is_empty() || auth.upload_key.is_empty() {
            return Err(terminal("web_rejected", "Registro de subida incompleto."));
        }
        let response = self
            .client
            .post(&auth.url)
            .header("Content-Type", &auth.content_type)
            .body(s3_multipart_body(&auth.prefix, file_bytes, &auth.suffix))
            .send()
            .map_err(|error| web_offline(error.to_string()))?;
        if !response.status().is_success() {
            return Err(terminal(
                "upload_failed",
                format!("El almacenamiento devolvió {}", response.status()),
            ));
        }
        Ok(())
    }

    fn finalize_upload(
        &self,
        key: &str,
        upload_key: &str,
        mtime_ms: u64,
    ) -> Result<(), IngestFailure> {
        let params = [
            ("upload", upload_key.to_string()),
            ("mtime", mtime_ms.to_string()),
        ];
        let response = self
            .client
            .post(format!("{}/{}/file", self.items_url(), key))
            .header("Zotero-API-Key", &self.api_key)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("If-None-Match", "*")
            .form(&params)
            .send()
            .map_err(|error| web_offline(error.to_string()))?;
        if !response.status().is_success() {
            return Err(terminal(
                "finalize_failed",
                format!("No se pudo confirmar la subida: {}", response.status()),
            ));
        }
        Ok(())
    }

    fn readback_attachment(
        &self,
        key: &str,
        expected_md5: &str,
    ) -> Result<IngestReceipt, IngestFailure> {
        let response = self
            .client
            .get(format!("{}/{}?format=json", self.items_url(), key))
            .header("Zotero-API-Key", &self.api_key)
            .send()
            .map_err(|error| web_offline(error.to_string()))?;
        classify(&response, "No se pudo releer el adjunto")?;
        let item: serde_json::Value = response
            .json()
            .map_err(|error| terminal("web_rejected", format!("Respuesta ilegible: {error}")))?;
        let version = item
            .get("version")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| terminal("readback_miss", "El adjunto aún no devuelve versión."))?;
        let remote_md5 = item
            .get("data")
            .and_then(|data| data.get("md5"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if remote_md5 != expected_md5 {
            return Err(terminal(
                "readback_miss",
                "El adjunto no trae el hash esperado.",
            ));
        }
        Ok(IngestReceipt {
            item_key: key.to_string(),
            version,
            library_external_id: self.group_id.clone(),
        })
    }
}

/// Reads a Web API JSON answer with its status, quoting the raw body
/// when it is not JSON — undiagnosable parse errors are a support trap.
fn read_json(
    response: reqwest::blocking::Response,
) -> Result<(u16, serde_json::Value), IngestFailure> {
    let status = response.status().as_u16();
    let text = response
        .text()
        .map_err(|error| terminal("web_rejected", format!("Respuesta ilegible: {error}")))?;
    let body: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
        terminal(
            "web_rejected",
            format!(
                "Zotero devolvió HTTP {status} no-JSON: {}",
                text.chars().take(160).collect::<String>()
            ),
        )
    })?;
    Ok((status, body))
}

fn web_offline(detail: String) -> IngestFailure {
    IngestFailure {
        terminal: false,
        code: "web_offline".to_string(),
        message: detail,
    }
}

/// Maps Web API failures: auth problems are terminal (retrying changes
/// nothing), everything else is resumable.
fn classify(response: &reqwest::blocking::Response, context: &str) -> Result<(), IngestFailure> {
    let status = response.status().as_u16();
    match status {
        200..=299 => Ok(()),
        401 => Err(terminal("invalid_key", "La clave de Zotero no es válida.")),
        403 => Err(terminal(
            "forbidden",
            "La clave no tiene escritura en ese grupo.",
        )),
        404 => Err(terminal(
            "unknown_item",
            "Ese registro ya no existe en Zotero.",
        )),
        _ => Err(IngestFailure {
            terminal: false,
            code: "web_rejected".to_string(),
            message: format!("{context}: HTTP {status}"),
        }),
    }
}

fn web_status(status: u16, body: &serde_json::Value) -> IngestFailure {
    match status {
        401 => terminal("invalid_key", "La clave de Zotero no es válida."),
        403 => terminal("forbidden", "La clave no tiene escritura en ese grupo."),
        404 => terminal("unknown_item", "Ese registro ya no existe en Zotero."),
        _ => {
            let detail = body
                .get("failed")
                .and_then(|value| value.get("0"))
                .and_then(|value| value.get("message"))
                .and_then(|value| value.as_str())
                .unwrap_or("sin detalle");
            IngestFailure {
                terminal: false,
                code: "web_rejected".to_string(),
                message: format!("Zotero devolvió {status}: {detail}"),
            }
        }
    }
}

impl IngestTransport for WebApiUploadTransport {
    fn execute(
        &self,
        conn: &mut Connection,
        operation: &IngestOperation,
    ) -> Result<IngestOutcome, IngestFailure> {
        if operation.kind.as_str() != KIND_UPLOAD_ATTACHMENT {
            return Err(terminal(
                "invalid_kind",
                "Ese transporte solo sube adjuntos.",
            ));
        }
        // The decision names an internal library row; uploads only run
        // when its external namespace is the targeted group — before any
        // HTTP happens.
        let decided_external: Option<String> = conn
            .query_row(
                "SELECT library_id FROM zotero_libraries WHERE id = ?1",
                [&operation.library_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| {
                terminal(
                    "sql_error",
                    format!("No se pudo resolver la biblioteca: {error}"),
                )
            })?;
        if decided_external.as_deref() != Some(self.group_id.as_str()) {
            return Err(terminal(
                "wrong_library",
                "Ese transporte solo escribe en el grupo objetivo.",
            ));
        }
        let plan = upload_plan_from_payload(conn, operation)?;
        let file_bytes = std::fs::read(&plan.file_path)
            .map_err(|_| terminal("file_missing", "El archivo ya no está en disco."))?;
        if file_bytes.len() as u64 > MAX_UPLOAD_BYTES {
            return Err(terminal(
                "file_too_large",
                format!("El archivo supera el máximo de {MAX_UPLOAD_BYTES} bytes."),
            ));
        }
        let md5_hex = md5_hex(&file_bytes);
        let mtime_ms = std::fs::metadata(&plan.file_path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or_else(|| super::super::processing::repository::now_ms() as u64);
        // Lost-response safety: same filename + hash under the parent
        // means a previous attempt landed — link it instead of duplicating.
        for child in self.remote_children(&plan.parent_item_key)? {
            let data = child.get("data");
            let same_file = data
                .and_then(|data| data.get("filename"))
                .and_then(|value| value.as_str())
                == Some(plan.filename.as_str());
            let same_hash = data
                .and_then(|data| data.get("md5"))
                .and_then(|value| value.as_str())
                == Some(md5_hex.as_str());
            if same_file && same_hash {
                let key = child.get("key").and_then(|v| v.as_str()).unwrap_or("");
                let version = child.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
                return Ok(IngestOutcome::Created(IngestReceipt {
                    item_key: key.to_string(),
                    version,
                    library_external_id: self.group_id.clone(),
                }));
            }
        }
        let key = self.create_attachment_item(&plan)?;
        let auth = self.register_upload(
            &key,
            &plan,
            &md5_hex,
            mtime_ms,
            file_bytes.len() as u64,
            &file_bytes,
        )?;
        self.post_to_s3(&auth, &file_bytes)?;
        self.finalize_upload(&key, &auth.upload_key, mtime_ms)?;
        Ok(IngestOutcome::Created(
            self.readback_attachment(&key, &md5_hex)?,
        ))
    }
}

/// MD5 hex digest (RFC 1321), dependency-free: the workspace lock is
/// frozen, and Zotero mandates MD5 for upload registration. Checksum
/// use only — never a security boundary.
pub fn md5_hex(bytes: &[u8]) -> String {
    let s: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, //
        5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, //
        4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, //
        6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    #[rustfmt::skip]
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
        0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
        0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
        0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
        0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
        0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
        0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
    ];
    let mut state: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut message = bytes.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_le_bytes());
    for chunk in message.chunks_exact(64) {
        let mut block = [0u32; 16];
        for (word, bytes) in block.iter_mut().zip(chunk.chunks_exact(4)) {
            *word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | ((!b) & d), i),
                16..=31 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | (!d)), (7 * i) % 16),
            };
            let sum = a.wrapping_add(f).wrapping_add(K[i]).wrapping_add(block[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(sum.rotate_left(s[i]));
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }
    let mut digest = String::with_capacity(32);
    for word in state {
        for byte in word.to_le_bytes() {
            digest.push_str(&format!("{byte:02x}"));
        }
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload_operation(payload: &str) -> IngestOperation {
        IngestOperation {
            id: "op-1".to_string(),
            request_id: "req-1".to_string(),
            kind: KIND_UPLOAD_ATTACHMENT.to_string(),
            library_id: "lib-1".to_string(),
            payload_json: payload.to_string(),
            state: "running".to_string(),
            attempt_count: 1,
            receipt_json: None,
            last_error_code: None,
            last_error_message: None,
        }
    }

    #[test]
    fn md5_matches_rfc_vectors() {
        for (input, expected) in [
            ("", "d41d8cd98f00b204e9800998ecf8427e"),
            ("a", "0cc175b9c0f1b6a831c399e269772661"),
            ("abc", "900150983cd24fb0d6963f7d28e17f72"),
            ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
            (
                "abcdefghijklmnopqrstuvwxyz",
                "c3fcd3d76192e4007dfb496cca67e13b",
            ),
            (
                "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
                "8215ef0796a20bcaaae116d3876c664a",
            ),
            (
                "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
                "d174ab98d277d9f5a5611c2c9f419d9f",
            ),
        ] {
            assert_eq!(md5_hex(input.as_bytes()), expected, "for {input:?}");
        }
    }

    #[test]
    fn content_types_map_by_extension() {
        assert_eq!(content_type_for_filename("tesis.pdf"), "application/pdf");
        assert_eq!(content_type_for_filename("NOTAS.PDF"), "application/pdf");
        assert_eq!(content_type_for_filename("foto.png"), "image/png");
        assert_eq!(content_type_for_filename("foto.jpg"), "image/jpeg");
        assert_eq!(content_type_for_filename("texto.txt"), "text/plain");
        assert_eq!(
            content_type_for_filename("raro.xyz"),
            "application/octet-stream"
        );
    }

    #[test]
    fn attachment_body_carries_link_mode_without_filesize() {
        let body = attachment_item_body("ABC12345", "zsb.pdf", "application/pdf");
        let item = body.get(0).expect("one attachment item");
        assert_eq!(
            item.get("linkMode").and_then(|v| v.as_str()),
            Some("imported_file")
        );
        assert_eq!(
            item.get("parentItem").and_then(|v| v.as_str()),
            Some("ABC12345")
        );
        assert!(item.get("filesize").is_none(), "the API rejects filesize");
        assert!(
            item.get("md5").is_none(),
            "md5 travels in the register step"
        );
    }

    #[test]
    fn s3_body_concatenates_in_order() {
        assert_eq!(s3_multipart_body(b"<", b"PDF", b">"), b"<PDF>".to_vec());
    }

    #[test]
    fn plan_resolves_parent_and_file() {
        let mut conn = plan_db();
        let (library_id, item_id) = seed_parent(&mut conn);
        let dir = std::env::temp_dir().join("zsb-upload-plan");
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("nota.pdf");
        std::fs::write(&file, b"%PDF-1.4 probe").expect("write");
        let op = IngestOperation {
            id: "op-1".to_string(),
            request_id: "req-1".to_string(),
            kind: KIND_UPLOAD_ATTACHMENT.to_string(),
            library_id: library_id.clone(),
            payload_json: format!(
                r#"{{"mode":"upload","item_id":"{item_id}","file_path":{}}}"#,
                serde_json::to_string(file.to_str().unwrap()).unwrap(),
            ),
            state: "running".to_string(),
            attempt_count: 1,
            receipt_json: None,
            last_error_code: None,
            last_error_message: None,
        };
        let plan = upload_plan_from_payload(&conn, &op).expect("plan");
        assert_eq!(plan.parent_item_key, "PARENT01");
        assert_eq!(plan.filename, "nota.pdf");
        assert_eq!(plan.content_type, "application/pdf");
        std::fs::remove_file(&file).ok();
    }

    #[test]
    fn transport_refuses_foreign_libraries_without_http() {
        let mut conn = plan_db();
        let (_library_id, item_id) = seed_parent(&mut conn);
        let op = IngestOperation {
            id: "op-1".to_string(),
            request_id: "req-1".to_string(),
            kind: KIND_UPLOAD_ATTACHMENT.to_string(),
            library_id: "other-lib".to_string(),
            payload_json: format!(
                r#"{{"mode":"upload","item_id":"{item_id}","file_path":"/tmp/x.pdf"}}"#,
            ),
            state: "queued".to_string(),
            attempt_count: 0,
            receipt_json: None,
            last_error_code: None,
            last_error_message: None,
        };
        let transport =
            WebApiUploadTransport::new("https://127.0.0.1:9", "6680944", "secret").expect("build");
        let failure = transport.execute(&mut conn, &op).expect_err("must refuse");
        assert_eq!(failure.code, "wrong_library");
        assert!(failure.terminal);
    }

    /// Live round-trip through the Web API against the isolated `prueba`
    /// group: parent probe → staged PDF → upload → md5-verified receipt.
    /// Runs only with `ZSB_LIVE_ZOTERO_WRITE=1` and the user-configured
    /// key file; leaves two labeled BORRAR rows for the manual cleanup.
    #[test]
    fn live_upload_round_trips_a_file() {
        if std::env::var("ZSB_LIVE_ZOTERO_WRITE").is_err() {
            eprintln!("skipping live web upload: ZSB_LIVE_ZOTERO_WRITE is not set");
            return;
        }
        let key = std::fs::read_to_string("C:/Users/agusn/.zsb/zotero_key.txt")
            .ok()
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty())
            .expect("key file");
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("http");
        let stamp = crate::processing::repository::now_ms();
        let parent: serde_json::Value = client
            .post("https://api.zotero.org/groups/6680944/items")
            .header("Zotero-API-Key", &key)
            .json(&serde_json::json!([{
                "itemType": "book",
                "title": format!("ZSB web transport BORRAR {stamp}"),
                "tags": [{"tag": "zsb-probe"}],
            }]))
            .send()
            .expect("create parent")
            .json()
            .expect("parent json");
        let parent_key = parent
            .get("successful")
            .and_then(|value| value.get("0"))
            .and_then(|value| value.get("key"))
            .and_then(|value| value.as_str())
            .expect("parent key")
            .to_string();

        let mut conn = plan_db();
        let library_id = seed_group_library(&mut conn);
        let item_id = {
            use crate::bibliography::repository::{upsert_item, BibliographicItemInput};
            upsert_item(
                &mut conn,
                &library_id,
                BibliographicItemInput {
                    item_key: parent_key.clone(),
                    item_version: Some(1),
                    native_json_snapshot: format!(r#"{{"key":"{parent_key}"}}"#),
                    csl_json_snapshot: format!(r#"{{"id":"{parent_key}"}}"#),
                    title: Some("ZSB web transport BORRAR".to_string()),
                    ..Default::default()
                },
            )
            .expect("seed parent")
            .id
        };
        let dir = std::env::temp_dir().join(format!("zsb-live-upload-{stamp}"));
        std::fs::create_dir_all(&dir).expect("dir");
        let file = dir.join("prueba.pdf");
        std::fs::write(
            &file,
            format!("%PDF-1.4 live transport probe {stamp}").into_bytes(),
        )
        .expect("stage pdf");
        let op = crate::bibliography::ingest::record_ingest_decision(
            &conn,
            &format!("req-live-upload-{stamp}"),
            &crate::bibliography::ingest::IngestDecision {
                kind: KIND_UPLOAD_ATTACHMENT.to_string(),
                library_id: library_id.clone(),
                payload_json: format!(
                    r#"{{"mode":"upload","item_id":"{item_id}","file_path":{}}}"#,
                    serde_json::to_string(file.to_str().unwrap()).unwrap(),
                ),
            },
        )
        .expect("record");
        let transport =
            WebApiUploadTransport::new("https://api.zotero.org", "6680944", &key).expect("build");
        let done = crate::bibliography::ingest::run_ingest_operation(&mut conn, &op.id, &transport)
            .expect("run live upload");
        assert_eq!(done.state, "succeeded");
        let receipt = done.receipt_json.expect("receipt");
        let parsed: serde_json::Value = serde_json::from_str(&receipt).expect("receipt json");
        assert_eq!(
            parsed
                .get("item_key")
                .and_then(|v| v.as_str())
                .map(str::len),
            Some(8)
        );
        assert!(parsed.get("version").and_then(|v| v.as_u64()).unwrap_or(0) >= 1);
        assert_eq!(
            parsed.get("library").and_then(|v| v.as_str()),
            Some("6680944")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    fn plan_db() -> Connection {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0040_bibliography_catalog.sql"
        ))
        .expect("catalog");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0041_bibliography_relations.sql"
        ))
        .expect("relations");
        conn.execute_batch(include_str!(
            "../../../../../packages/store/src/migrations/0056_bibliographic_ingest_operations.sql"
        ))
        .expect("tray");
        conn
    }

    fn seed_parent(conn: &mut Connection) -> (String, String) {
        use crate::bibliography::repository::{
            upsert_connection, upsert_item, upsert_library, BibliographicItemInput, LibraryType,
            SourceOrigin, UpsertConnection, UpsertLibrary,
        };
        let source = upsert_connection(
            conn,
            UpsertConnection {
                id: "conn-1".to_string(),
                source_origin: SourceOrigin::Local,
                source_instance_id: None,
                endpoint: Some("http://synthetic.invalid".to_string()),
                capabilities_json: r#"{"read":true}"#.to_string(),
            },
        )
        .expect("connection");
        let library = upsert_library(
            conn,
            UpsertLibrary {
                connection_id: source.id,
                library_type: LibraryType::Group,
                library_id: "6680944".to_string(),
                name: "prueba".to_string(),
                last_modified_version: None,
            },
        )
        .expect("library")
        .id;
        let item = upsert_item(
            conn,
            &library,
            BibliographicItemInput {
                item_key: "PARENT01".to_string(),
                item_version: Some(3),
                native_json_snapshot: r#"{"key":"PARENT01","version":3}"#.to_string(),
                csl_json_snapshot: r#"{"id":"PARENT01","title":"Padre"}"#.to_string(),
                title: Some("Padre".to_string()),
                ..Default::default()
            },
        )
        .expect("parent")
        .id;
        (library, item)
    }

    fn seed_group_library(conn: &mut Connection) -> String {
        seed_parent(conn).0
    }

    #[test]
    fn plan_rejects_bad_payloads_before_network() {
        let conn = Connection::open_in_memory().expect("memory db");
        for (payload, code) in [
            (r#"{"mode":"upload"}"#, "invalid_payload"),
            (r#"{"mode":"upload","item_id":"x"}"#, "invalid_payload"),
            (
                r#"{"mode":"link","item_id":"x","file_path":"f"}"#,
                "invalid_payload",
            ),
            ("not json", "invalid_payload"),
        ] {
            let error =
                upload_plan_from_payload(&conn, &upload_operation(payload)).expect_err("must fail");
            assert_eq!(error.code, code, "for {payload}");
            assert!(error.terminal);
        }
    }
}
