//! Read-only web tools for the help agent (T23, S16, D14): `repo_tree`,
//! `repo_read` and `hlab_entropia_page`.
//!
//! The model answers questions about EntropIA that the manual does not
//! cover (Lite vs Pro, requirements, releases): the HLab page first, then
//! the public repo (`HumaLab/EntropIA-Pro-Lite`, live). Only three URL
//! shapes are ever fetched (see [`is_allowed_url`]); redirects to other
//! hosts are refused. Tests inject [`test_support::MockFetch`]; production
//! uses reqwest with a 15 s timeout. No live network in tests.
//!
//! Cache design: each [`WebTools`] keeps the repo tree for 10 minutes and
//! the HLab text for 30 minutes behind a plain `Mutex` (no async lock is
//! needed: entries are cloned out before any await). One executor lives for
//! one `help_ask`, so every tool call of that question shares the downloads
//! while the TTLs bound staleness.
//!
//! HTML-to-text approach: reuse, not a new stripper. The HLab page goes
//! through `bibliography::html_text` — the same `dom_query` extractor the
//! app uses for stored pages — which drops `script`/`style`/`nav`/`header`/
//! `footer` and hidden nodes and keeps one paragraph per block (headings and
//! link text included), after `decode_html_bytes` honours BOM and `<meta
//! charset>`. The result is then capped at [`PAGE_MAX_CHARS`].

use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::agent_llm::{ToolOutcome, ToolSpec};

/// The full GitHub trees URL of the default branch, unauthenticated.
pub(crate) const GITHUB_API_TREE_URL: &str =
    "https://api.github.com/repos/HumaLab/EntropIA-Pro-Lite/git/trees/HEAD?recursive=1";
/// Raw file downloads live under this prefix; the repo path is appended.
pub(crate) const GITHUB_RAW_BASE: &str =
    "https://raw.githubusercontent.com/HumaLab/EntropIA-Pro-Lite/HEAD/";
/// The only HLab page the assistant may read.
pub(crate) const HLAB_PAGE_URL: &str = "https://hlab.com.ar/herramientas/entropia-lite";

/// How long the repo tree (10 min) and the HLab page (30 min) are reused.
pub(crate) const TREE_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
pub(crate) const PAGE_CACHE_TTL: Duration = Duration::from_secs(30 * 60);
/// At most this many tree entries per answer; the rest is counted, not listed.
pub(crate) const TREE_MAX_ENTRIES: usize = 300;
/// At most this many lines and chars of a repo file per answer.
pub(crate) const READ_MAX_LINES: usize = 400;
pub(crate) const READ_MAX_CHARS: usize = 40_000;
/// Files bigger than this are refused before and after download.
pub(crate) const READ_MAX_BYTES: usize = 1_000_000;
/// At most this many chars of the HLab page per answer.
pub(crate) const PAGE_MAX_CHARS: usize = 30_000;
/// A stalled fetch fails fast: same budget as the help OpenRouter client.
pub(crate) const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// HTTP layer (injected for tests)
// ---------------------------------------------------------------------------

/// One fetched answer: the status, the URL after redirects, and the raw bytes.
#[derive(Clone, Debug)]
pub(crate) struct HttpResponse {
    pub status: u16,
    pub final_url: String,
    pub body: Vec<u8>,
}

/// The HTTP layer: production uses reqwest, tests a scripted mock.
pub(crate) trait HttpFetch: Send + Sync {
    fn fetch(
        &self,
        url: &str,
    ) -> impl std::future::Future<Output = Result<HttpResponse, String>> + Send;
}

/// The reqwest HTTP layer (D14): `User-Agent: EntropIA` per
/// [`crate::USER_AGENT`], `Accept: application/vnd.github+json` on the API,
/// 15 s timeouts. Redirects are followed by reqwest; the caller checks the
/// final URL against the allowlist.
pub(crate) struct ReqwestFetch {
    client: reqwest::Client,
}

impl Default for ReqwestFetch {
    fn default() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(crate::USER_AGENT)
            .timeout(HTTP_TIMEOUT)
            .connect_timeout(HTTP_TIMEOUT)
            .build()
            .expect("Failed to build help web reqwest client");
        Self { client }
    }
}

impl HttpFetch for ReqwestFetch {
    async fn fetch(&self, url: &str) -> Result<HttpResponse, String> {
        let mut request = self.client.get(url);
        if url.starts_with("https://api.github.com/") {
            request = request.header("Accept", "application/vnd.github+json");
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("No se pudo contactar al servidor: {error}"))?;
        let status = response.status().as_u16();
        let final_url = response.url().to_string();
        let body = response
            .bytes()
            .await
            .map_err(|error| format!("No se pudo leer la respuesta: {error}"))?
            .to_vec();
        Ok(HttpResponse {
            status,
            final_url,
            body,
        })
    }
}

// ---------------------------------------------------------------------------
// Allowlist
// ---------------------------------------------------------------------------

/// Whether this exact URL may be fetched: only the three hosts and only the
/// three shapes (the GitHub tree of the repo, raw files under its `HEAD`,
/// the HLab EntropIA page). Anything else — including redirects checked
/// afterwards — is refused.
pub(crate) fn is_allowed_url(url: &str) -> bool {
    let parsed = match url::Url::parse(url) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };
    if parsed.scheme() != "https" {
        return false;
    }
    match parsed.host_str() {
        Some("api.github.com") => {
            parsed.path() == "/repos/HumaLab/EntropIA-Pro-Lite/git/trees/HEAD"
        }
        Some("raw.githubusercontent.com") => parsed
            .path()
            .strip_prefix("/HumaLab/EntropIA-Pro-Lite/HEAD/")
            .is_some_and(|rest| !rest.is_empty()),
        Some("hlab.com.ar") => {
            parsed.path() == "/herramientas/entropia-lite"
                || parsed.path() == "/herramientas/entropia-lite/"
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Repo paths
// ---------------------------------------------------------------------------

/// Whether the executor offers `name` as a web tool.
pub(crate) fn is_web_tool(name: &str) -> bool {
    matches!(name, "repo_tree" | "repo_read" | "hlab_entropia_page")
}

/// The three tool specs offered to the model alongside the MCP tools.
pub(crate) fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "repo_tree".to_string(),
            description: "Lista archivos del repositorio público HumaLab/EntropIA-Pro-Lite (rama por defecto). Pasá path_prefix para filtrar.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path_prefix": {"type": "string", "description": "Prefijo de ruta para filtrar, por ejemplo docs/."},
                },
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "repo_read".to_string(),
            description: "Lee un archivo del repositorio público HumaLab/EntropIA-Pro-Lite. Solo rutas del árbol publicado.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Ruta del archivo dentro del repo, por ejemplo README.md."},
                    "from_line": {"type": "integer", "description": "Primera línea (1-based)."},
                    "to_line": {"type": "integer", "description": "Última línea incluida."},
                },
                "required": ["path"],
                "additionalProperties": false,
            }),
        },
        ToolSpec {
            name: "hlab_entropia_page".to_string(),
            description: "Lee la página de EntropIA en el sitio de HLab como texto.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false,
            }),
        },
    ]
}

/// One file of the repo tree: its path and its blob size, when known.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TreeEntry {
    path: String,
    size: Option<u64>,
}

/// The checked repo path, or why it cannot be read: `..` segments, absolute
/// paths, backslashes, URL schemes and empty paths are all refused.
pub(crate) fn validate_repo_path(path: &str) -> Result<String, String> {
    if path.is_empty() {
        return Err("La ruta está vacía.".to_string());
    }
    if path.contains('\\') {
        return Err(format!("«{path}» no es válida: usá barras «/», sin «\\»."));
    }
    if path.starts_with('/') {
        return Err(format!(
            "«{path}» no es válida: la ruta debe ser relativa al repositorio."
        ));
    }
    if path.split('/').any(|segment| segment == "..") {
        return Err(format!(
            "«{path}» no es válida: no se permite subir con «..»."
        ));
    }
    if path.contains("://") || has_url_scheme(path) {
        return Err(format!(
            "«{path}» no es válida: no se permiten URLs, solo rutas del repositorio."
        ));
    }
    Ok(path.to_string())
}

/// Whether `path` starts with a `scheme:` prefix (`https:`, `C:`, …): the
/// scheme is ASCII alphanumeric plus `+`, `-`, `.`, like URLs use.
fn has_url_scheme(path: &str) -> bool {
    let Some(colon) = path.find(':') else {
        return false;
    };
    let (scheme, _) = path.split_at(colon);
    !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

#[derive(serde::Deserialize)]
struct TreesResponse {
    #[serde(default)]
    tree: Vec<TreeNode>,
}

#[derive(serde::Deserialize)]
struct TreeNode {
    #[serde(default)]
    path: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

/// The blob entries of a GitHub trees answer: only `type == "blob"` with a
/// path; directories and submodules never reach the model.
pub(crate) fn parse_tree_entries(body: &[u8]) -> Result<Vec<TreeEntry>, String> {
    let parsed: TreesResponse = serde_json::from_slice(body)
        .map_err(|error| format!("No se pudo leer el árbol del repositorio: {error}"))?;
    Ok(parsed
        .tree
        .into_iter()
        .filter_map(|node| {
            let path = node.path?;
            if node.kind.as_deref() != Some("blob") || path.is_empty() {
                return None;
            }
            Some(TreeEntry {
                path,
                size: node.size,
            })
        })
        .collect())
}

/// The entries under `prefix` (empty means all), at most `limit`: the shown
/// entries plus how many matched but were cut.
pub(crate) fn filter_tree<'a>(
    entries: &'a [TreeEntry],
    prefix: &str,
    limit: usize,
) -> (Vec<&'a TreeEntry>, usize) {
    let matched: Vec<&TreeEntry> = entries
        .iter()
        .filter(|entry| entry.path.starts_with(prefix))
        .collect();
    let omitted = matched.len().saturating_sub(limit);
    (matched.into_iter().take(limit).collect(), omitted)
}

/// The formatted tree answer: one `- path (N bytes)` line per shown entry
/// and an `… N más` note when the cap cut the rest.
fn render_tree(prefix: &str, shown: &[&TreeEntry], omitted: usize, total: usize) -> String {
    if shown.is_empty() && !prefix.is_empty() {
        return format!(
            "Sin archivos con el prefijo «{prefix}» en el árbol de HumaLab/EntropIA-Pro-Lite."
        );
    }
    let mut out = if prefix.is_empty() {
        format!("Árbol de HumaLab/EntropIA-Pro-Lite (rama por defecto): {total} archivos:")
    } else {
        format!(
            "Árbol de HumaLab/EntropIA-Pro-Lite (rama por defecto) con prefijo «{prefix}»: {total} archivos:"
        )
    };
    for entry in shown {
        match entry.size {
            Some(size) => out.push_str(&format!("\n- {} ({size} bytes)", entry.path)),
            None => out.push_str(&format!("\n- {}", entry.path)),
        }
    }
    if omitted > 0 {
        out.push_str(&format!(
            "\n… {omitted} más: pedí con un path_prefix más específico para ver el resto."
        ));
    }
    out
}

/// One slice of a text file: the 1-based lines `from..=to` of `total_lines`,
/// capped at [`READ_MAX_LINES`] lines and [`READ_MAX_CHARS`] chars.
pub(crate) struct TextSlice {
    pub from: usize,
    pub to: usize,
    pub total_lines: usize,
    pub snippet: String,
    pub has_more: bool,
    pub char_truncated: bool,
}

/// Cuts `text` to the requested range within the per-answer caps. `from_line`
/// below 1 reads from the start; the caller rejects a `to_line` before
/// `from_line` as invalid, and an empty file yields an empty slice.
pub(crate) fn slice_text_lines(text: &str, from_line: usize, to_line: Option<usize>) -> TextSlice {
    let lines: Vec<&str> = text.lines().collect();
    let total_lines = lines.len();
    if total_lines == 0 {
        return TextSlice {
            from: 1,
            to: 0,
            total_lines: 0,
            snippet: String::new(),
            has_more: false,
            char_truncated: false,
        };
    }
    let from = from_line.max(1).min(total_lines);
    let capped = (from + READ_MAX_LINES - 1).min(total_lines);
    let end = match to_line {
        Some(to) => to.min(capped),
        None => capped,
    }
    .max(from - 1);
    let joined = lines[from - 1..end].join("\n");
    let (snippet, char_truncated) = truncate_chars(&joined, READ_MAX_CHARS);
    TextSlice {
        from,
        to: end,
        total_lines,
        snippet,
        has_more: end < total_lines,
        char_truncated,
    }
}

/// The first `max` chars of `text`, and whether anything was cut.
fn truncate_chars(text: &str, max: usize) -> (String, bool) {
    if text.chars().count() <= max {
        return (text.to_string(), false);
    }
    (text.chars().take(max).collect(), true)
}

/// The formatted file answer: a `«path» (líneas X–Y de Z)` header and, when
/// the file continues, which `from_line` reads the next range.
fn render_slice(path: &str, slice: &TextSlice) -> String {
    if slice.total_lines == 0 {
        return format!("«{path}» (vacío).");
    }
    let mut out = format!(
        "«{path}» (líneas {}–{} de {}):\n{}",
        slice.from, slice.to, slice.total_lines, slice.snippet
    );
    if slice.has_more {
        let next = slice.to + 1;
        out.push_str(&format!(
            "\n… continúa en la línea {next}: pedí repo_read con from_line={next} para seguir leyendo."
        ));
    }
    if slice.char_truncated {
        out.push_str("\n… (texto cortado en 40 000 caracteres)");
    }
    out
}

/// The readable text of the HLab page: paragraphs without scripts, styles or
/// navigation, headings and link text kept (see the module docs).
pub(crate) fn html_to_readable_text(html: &str) -> String {
    crate::bibliography::html_text::html_to_paragraphs(html)
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// The read-only web tools over an injected [`HttpFetch`] layer: the tree
/// and the page are cached in memory (10 and 30 minutes).
pub(crate) struct WebTools<H = ReqwestFetch> {
    fetcher: H,
    tree_cache: Mutex<Option<(Instant, Vec<TreeEntry>)>>,
    page_cache: Mutex<Option<(Instant, String)>>,
}

impl<H: HttpFetch> WebTools<H> {
    pub(crate) fn new(fetcher: H) -> Self {
        Self {
            fetcher,
            tree_cache: Mutex::new(None),
            page_cache: Mutex::new(None),
        }
    }

    /// Fetches `url` after the allowlist check and refuses redirects off the
    /// allowed hosts, HTTP errors (403 from GitHub names the rate limit in
    /// Spanish) and anything else that is not a 200 body.
    async fn fetch_checked(&self, url: &str) -> Result<Vec<u8>, String> {
        if !is_allowed_url(url) {
            return Err(
                "URL no permitida: solo se puede leer el repositorio público y la página de EntropIA en HLab."
                    .to_string(),
            );
        }
        let response = self.fetcher.fetch(url).await?;
        if !is_allowed_url(&response.final_url) {
            return Err(format!(
                "Redirección no permitida fuera de los hosts de ayuda: {}",
                response.final_url
            ));
        }
        if response.status == 403 && url.starts_with("https://api.github.com/") {
            return Err(
                "Límite de GitHub alcanzado (403): la API no permite más consultas sin autenticación; esperá unos minutos y reintentá."
                    .to_string(),
            );
        }
        if response.status != 200 {
            return Err(format!(
                "La descarga devolvió un error ({})",
                response.status
            ));
        }
        Ok(response.body)
    }

    /// The cached repo tree, downloaded once while fresh.
    async fn cached_tree(&self) -> Result<Vec<TreeEntry>, String> {
        let fresh = self
            .tree_cache
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or(None)
            .filter(|(at, _)| at.elapsed() < TREE_CACHE_TTL)
            .map(|(_, entries)| entries);
        if let Some(entries) = fresh {
            return Ok(entries);
        }
        let body = self.fetch_checked(GITHUB_API_TREE_URL).await?;
        let entries = parse_tree_entries(&body)?;
        if let Ok(mut guard) = self.tree_cache.lock() {
            *guard = Some((Instant::now(), entries.clone()));
        }
        Ok(entries)
    }

    /// Dispatches one web tool call: `Ok` carries the text for the model,
    /// anything else explains in Spanish why there is no text.
    pub(crate) async fn call(&self, name: &str, args: serde_json::Value) -> ToolOutcome {
        match name {
            "repo_tree" => self.repo_tree(&args).await,
            "repo_read" => self.repo_read(&args).await,
            "hlab_entropia_page" => self.hlab_page().await,
            _ => ToolOutcome::Denied {
                reason: format!("herramienta desconocida '{name}'"),
            },
        }
    }

    async fn repo_tree(&self, args: &serde_json::Value) -> ToolOutcome {
        let prefix = args
            .get("path_prefix")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        match self.cached_tree().await {
            Ok(entries) => {
                let (shown, omitted) = filter_tree(&entries, prefix, TREE_MAX_ENTRIES);
                let total = shown.len() + omitted;
                ToolOutcome::Ok {
                    content: render_tree(prefix, &shown, omitted, total),
                }
            }
            Err(message) => ToolOutcome::Failed { message },
        }
    }

    async fn repo_read(&self, args: &serde_json::Value) -> ToolOutcome {
        let path = args
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let checked = match validate_repo_path(path) {
            Ok(checked) => checked,
            Err(reason) => return ToolOutcome::Failed { message: reason },
        };
        let from = args
            .get("from_line")
            .and_then(|value| value.as_u64())
            .unwrap_or(1)
            .max(1) as usize;
        let to = args
            .get("to_line")
            .and_then(|value| value.as_u64())
            .map(|value| value as usize);
        if to.is_some_and(|to| to < from) {
            return ToolOutcome::Failed {
                message: format!(
                    "El rango es inválido: to_line ({}) es menor que from_line ({from}).",
                    to.unwrap_or(0)
                ),
            };
        }
        let entries = match self.cached_tree().await {
            Ok(entries) => entries,
            Err(message) => return ToolOutcome::Failed { message },
        };
        let Some(entry) = entries.iter().find(|entry| entry.path == checked) else {
            return ToolOutcome::Failed {
                message: format!("«{checked}» no existe en el árbol del repositorio."),
            };
        };
        if entry.size.is_some_and(|size| size > READ_MAX_BYTES as u64) {
            return ToolOutcome::Failed {
                message: format!(
                    "«{checked}» es demasiado grande ({} bytes, máximo 1 000 000): el árbol ya dice su tamaño, pedí un archivo más chico.",
                    entry.size.unwrap_or(0)
                ),
            };
        }
        let url = format!("{GITHUB_RAW_BASE}{checked}");
        let body = match self.fetch_checked(&url).await {
            Ok(body) => body,
            Err(message) => return ToolOutcome::Failed { message },
        };
        if body.len() > READ_MAX_BYTES {
            return ToolOutcome::Failed {
                message: format!(
                    "«{checked}» es demasiado grande ({} bytes, máximo 1 000 000): pedí un rango menor o un archivo más chico.",
                    body.len()
                ),
            };
        }
        if body.contains(&0) {
            return ToolOutcome::Failed {
                message: format!("«{checked}» parece binario y no se puede leer como texto."),
            };
        }
        let text = String::from_utf8_lossy(&body).into_owned();
        let slice = slice_text_lines(&text, from, to);
        ToolOutcome::Ok {
            content: render_slice(&checked, &slice),
        }
    }

    async fn hlab_page(&self) -> ToolOutcome {
        let fresh = self
            .page_cache
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or(None)
            .filter(|(at, _)| at.elapsed() < PAGE_CACHE_TTL)
            .map(|(_, text)| text);
        if let Some(text) = fresh {
            return ToolOutcome::Ok { content: text };
        }
        let body = match self.fetch_checked(HLAB_PAGE_URL).await {
            Ok(body) => body,
            Err(message) => return ToolOutcome::Failed { message },
        };
        let html = crate::bibliography::html_text::decode_html_bytes(&body);
        let mut text = html_to_readable_text(&html);
        if text.chars().count() > PAGE_MAX_CHARS {
            let (cut, _) = truncate_chars(&text, PAGE_MAX_CHARS);
            text = format!("{cut}… (texto recortado a 30 000 caracteres)");
        }
        if let Ok(mut guard) = self.page_cache.lock() {
            *guard = Some((Instant::now(), text.clone()));
        }
        ToolOutcome::Ok { content: text }
    }
}

// ---------------------------------------------------------------------------
// Test support
// ---------------------------------------------------------------------------

/// The scripted HTTP layer for tests: program one answer per URL, then
/// inspect what was requested. Unprogrammed URLs fail, never touch the net.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    pub(crate) struct MockFetch {
        responses: Mutex<HashMap<String, HttpResponse>>,
        requested: Arc<Mutex<Vec<String>>>,
    }

    impl MockFetch {
        pub(crate) fn new() -> Self {
            Self {
                responses: Mutex::new(HashMap::new()),
                requested: Arc::new(Mutex::new(Vec::new())),
            }
        }

        pub(crate) fn respond(&self, url: &str, status: u16, final_url: &str, body: &[u8]) {
            self.responses.lock().expect("lock").insert(
                url.to_string(),
                HttpResponse {
                    status,
                    final_url: final_url.to_string(),
                    body: body.to_vec(),
                },
            );
        }

        /// The request log, shared so tests keep reading it after the mock
        /// moves into the executor.
        pub(crate) fn log(&self) -> Arc<Mutex<Vec<String>>> {
            Arc::clone(&self.requested)
        }
    }

    impl HttpFetch for MockFetch {
        async fn fetch(&self, url: &str) -> Result<HttpResponse, String> {
            self.requested.lock().expect("lock").push(url.to_string());
            self.responses
                .lock()
                .expect("lock")
                .get(url)
                .cloned()
                .ok_or_else(|| format!("URL no programada en el mock: {url}"))
        }
    }

    /// A GitHub trees answer with the given `(path, size)` blobs.
    pub(crate) fn tree_body(entries: &[(&str, Option<u64>)]) -> Vec<u8> {
        let tree: Vec<serde_json::Value> = entries
            .iter()
            .map(|(path, size)| {
                let mut node = serde_json::json!({"path": path, "type": "blob"});
                if let Some(size) = size {
                    node["size"] = serde_json::json!(size);
                }
                node
            })
            .collect();
        serde_json::json!({"sha": "abc", "truncated": false, "tree": tree})
            .to_string()
            .into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{tree_body, MockFetch};
    use super::*;

    fn count(log: &std::sync::Arc<Mutex<Vec<String>>>, url: &str) -> usize {
        log.lock()
            .expect("lock")
            .iter()
            .filter(|got| got.as_str() == url)
            .count()
    }

    #[test]
    fn tool_specs_names_the_three_read_tools() {
        let names: Vec<String> = tool_specs().iter().map(|tool| tool.name.clone()).collect();
        assert_eq!(names, vec!["repo_tree", "repo_read", "hlab_entropia_page"]);
        for tool in tool_specs() {
            assert!(
                !tool.description.trim().is_empty(),
                "{} lleva descripción",
                tool.name
            );
            assert_eq!(
                tool.input_schema.get("type"),
                Some(&serde_json::Value::String("object".to_string())),
                "{} publica un esquema objeto",
                tool.name
            );
        }
    }

    #[test]
    fn allowlist_only_covers_the_three_shapes() {
        assert!(is_allowed_url(GITHUB_API_TREE_URL));
        assert!(is_allowed_url(&format!("{GITHUB_RAW_BASE}README.md")));
        assert!(is_allowed_url(&format!("{GITHUB_RAW_BASE}docs/pagina.md")));
        assert!(is_allowed_url(HLAB_PAGE_URL));
        for evil in [
            "https://evil.example/robo",
            "https://api.github.com/repos/otro/repo/git/trees/HEAD?recursive=1",
            "https://raw.githubusercontent.com/otro/repo/HEAD/README.md",
            "https://hlab.com.ar/otra-pagina",
            "http://hlab.com.ar/herramientas/entropia-lite",
            "file:///etc/passwd",
        ] {
            assert!(!is_allowed_url(evil), "{evil} debe estar prohibida");
        }
    }

    #[test]
    fn allowlist_refuses_the_verify_report_cases() {
        // T32/S16 follow-up: userinfo-smuggled host and `..` escape are
        // refused. Hosts are case-insensitive (DNS), so the upper-case host
        // below is accepted only because the URL is otherwise exact — same
        // scheme, same host after lower-casing, same case-sensitive path —
        // which is safe: it fetches the same public file as the lower-case
        // form. The test pins that safe behaviour.
        for evil in [
            "https://raw.githubusercontent.com@evil.example/HumaLab/EntropIA-Pro-Lite/HEAD/README.md",
            "https://raw.githubusercontent.com/HumaLab/EntropIA-Pro-Lite/HEAD/../../secret",
        ] {
            assert!(!is_allowed_url(evil), "{evil} debe estar prohibida");
        }
        assert!(
            is_allowed_url(
                "https://RAW.GITHUBUSERCONTENT.com/HumaLab/EntropIA-Pro-Lite/HEAD/README.md"
            ),
            "el host en mayúsculas con la ruta exacta es el mismo archivo público"
        );
    }

    #[test]
    fn repo_paths_reject_escapes_schemes_and_absolute_paths() {
        for bad in [
            "../x",
            "docs/../../x",
            "/etc/passwd",
            "C:\\x",
            "https://evil",
        ] {
            assert!(validate_repo_path(bad).is_err(), "{bad} debe rechazarse");
        }
        assert!(validate_repo_path("").is_err(), "vacía debe rechazarse");
        assert_eq!(
            validate_repo_path("docs/pagina.md").expect("relativa válida"),
            "docs/pagina.md"
        );
    }

    #[test]
    fn tree_parsing_keeps_blobs_with_sizes() {
        let body = serde_json::json!({
            "tree": [
                {"path": "README.md", "type": "blob", "size": 12},
                {"path": "docs", "type": "tree"},
                {"path": "docs/a.md", "type": "blob"},
            ],
        })
        .to_string()
        .into_bytes();
        let entries = parse_tree_entries(&body).expect("parse");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "README.md");
        assert_eq!(entries[0].size, Some(12));
        assert_eq!(entries[1].size, None);
    }

    #[test]
    fn tree_filtering_applies_prefix_and_cap() {
        let entries = vec![
            TreeEntry {
                path: "docs/a.md".to_string(),
                size: None,
            },
            TreeEntry {
                path: "docs/b.md".to_string(),
                size: None,
            },
            TreeEntry {
                path: "README.md".to_string(),
                size: None,
            },
        ];
        let (shown, omitted) = filter_tree(&entries, "docs/", 300);
        assert_eq!(shown.len(), 2);
        assert_eq!(omitted, 0);
        let (shown, omitted) = filter_tree(&entries, "", 2);
        assert_eq!(shown.len(), 2);
        assert_eq!(omitted, 1);
        let (shown, omitted) = filter_tree(&entries, "nada/", 300);
        assert!(shown.is_empty());
        assert_eq!(omitted, 0);
    }

    #[test]
    fn slice_cuts_ranges_within_line_and_char_caps() {
        let text = (1..=10)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let slice = slice_text_lines(&text, 3, Some(5));
        assert_eq!((slice.from, slice.to, slice.total_lines), (3, 5, 10));
        assert_eq!(slice.snippet, "l3\nl4\nl5");
        assert!(slice.has_more);

        let big = (1..=500)
            .map(|i| format!("línea {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let slice = slice_text_lines(&big, 1, None);
        assert_eq!(slice.snippet.lines().count(), READ_MAX_LINES);
        assert_eq!((slice.from, slice.to), (1, 400));
        assert!(slice.has_more);

        let wide = vec!["x".repeat(20_000); 5].join("\n");
        let slice = slice_text_lines(&wide, 1, None);
        assert!(slice.snippet.chars().count() <= READ_MAX_CHARS);
        assert!(slice.char_truncated);
    }

    #[test]
    fn html_extraction_drops_furniture_and_keeps_headings_and_links() {
        let html = r#"<!doctype html><html><head><title>T</title>
            <style>p { color: red }</style><script>var secreto = 1;</script></head>
            <body><nav><a href="/">menú</a></nav>
            <h1>EntropIA Lite</h1>
            <p>Organizá tus documentos. <a href="/planes">Ver planes</a>.</p>
            </body></html>"#;
        let text = html_to_readable_text(html);
        assert!(text.contains("EntropIA Lite"), "{text}");
        assert!(text.contains("Ver planes"), "{text}");
        assert!(!text.contains("secreto"), "{text}");
        assert!(!text.contains("menú"), "{text}");
    }

    #[tokio::test]
    async fn tree_answer_lists_files_with_sizes_and_counts_the_rest() {
        let entries: Vec<(String, Option<u64>)> = (0..302)
            .map(|i| (format!("doc{i:03}.md"), Some(i as u64)))
            .collect();
        let borrowed: Vec<(&str, Option<u64>)> = entries
            .iter()
            .map(|(path, size)| (path.as_str(), *size))
            .collect();
        let fetch = MockFetch::new();
        fetch.respond(
            GITHUB_API_TREE_URL,
            200,
            GITHUB_API_TREE_URL,
            &tree_body(&borrowed),
        );
        let tools = WebTools::new(fetch);
        match tools.call("repo_tree", serde_json::json!({})).await {
            ToolOutcome::Ok { content } => {
                assert!(content.contains("doc000.md"), "{content}");
                assert!(content.contains("doc000.md (0 bytes)"), "{content}");
                assert!(content.contains("… 2 más"), "{content}");
            }
            other => panic!("el árbol debe listarse: {other:?}"),
        }
    }

    #[tokio::test]
    async fn tree_prefix_narrows_the_answer() {
        let fetch = MockFetch::new();
        fetch.respond(
            GITHUB_API_TREE_URL,
            200,
            GITHUB_API_TREE_URL,
            &tree_body(&[
                ("docs/a.md", Some(1)),
                ("docs/b.md", Some(2)),
                ("README.md", Some(3)),
            ]),
        );
        let tools = WebTools::new(fetch);
        match tools
            .call("repo_tree", serde_json::json!({"path_prefix": "docs/"}))
            .await
        {
            ToolOutcome::Ok { content } => {
                assert!(content.contains("docs/a.md"), "{content}");
                assert!(!content.contains("README.md"), "{content}");
                assert!(!content.contains("más"), "{content}");
            }
            other => panic!("el prefijo debe filtrar: {other:?}"),
        }
    }

    #[tokio::test]
    async fn tree_answer_is_reused_from_the_cache() {
        let fetch = MockFetch::new();
        fetch.respond(
            GITHUB_API_TREE_URL,
            200,
            GITHUB_API_TREE_URL,
            &tree_body(&[("README.md", Some(3))]),
        );
        let log = fetch.log();
        let tools = WebTools::new(fetch);
        for prefix in ["", "docs/"] {
            match tools
                .call("repo_tree", serde_json::json!({"path_prefix": prefix}))
                .await
            {
                ToolOutcome::Ok { .. } => {}
                other => panic!("el árbol debe listarse: {other:?}"),
            }
        }
        // Both prefixes share the one download: the cache, not the net.
        assert_eq!(count(&log, GITHUB_API_TREE_URL), 1);
    }

    #[tokio::test]
    async fn read_returns_the_requested_line_range() {
        let text = (1..=10)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let fetch = MockFetch::new();
        fetch.respond(
            GITHUB_API_TREE_URL,
            200,
            GITHUB_API_TREE_URL,
            &tree_body(&[("notas.txt", Some(30))]),
        );
        let url = format!("{GITHUB_RAW_BASE}notas.txt");
        fetch.respond(&url, 200, &url, text.as_bytes());
        let tools = WebTools::new(fetch);
        match tools
            .call(
                "repo_read",
                serde_json::json!({"path": "notas.txt", "from_line": 3, "to_line": 5}),
            )
            .await
        {
            ToolOutcome::Ok { content } => {
                assert!(content.contains("líneas 3–5 de 10"), "{content}");
                assert!(content.contains("l3"), "{content}");
                assert!(!content.contains("l6"), "{content}");
                assert!(content.contains("from_line=6"), "{content}");
            }
            other => panic!("la lectura debe paginar: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_caps_long_files_and_names_the_next_range() {
        let big = (1..=500)
            .map(|i| format!("línea {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let fetch = MockFetch::new();
        fetch.respond(
            GITHUB_API_TREE_URL,
            200,
            GITHUB_API_TREE_URL,
            &tree_body(&[("grande.txt", Some(5_000))]),
        );
        let url = format!("{GITHUB_RAW_BASE}grande.txt");
        fetch.respond(&url, 200, &url, big.as_bytes());
        let tools = WebTools::new(fetch);
        match tools
            .call("repo_read", serde_json::json!({"path": "grande.txt"}))
            .await
        {
            ToolOutcome::Ok { content } => {
                assert!(content.contains("líneas 1–400 de 500"), "{content}");
                assert!(content.contains("from_line=401"), "{content}");
            }
            other => panic!("la lectura debe capar: {other:?}"),
        }

        let wide = vec!["x".repeat(20_000); 5].join("\n");
        let fetch = MockFetch::new();
        fetch.respond(
            GITHUB_API_TREE_URL,
            200,
            GITHUB_API_TREE_URL,
            &tree_body(&[("ancho.txt", Some(100_000))]),
        );
        let url = format!("{GITHUB_RAW_BASE}ancho.txt");
        fetch.respond(&url, 200, &url, wide.as_bytes());
        let tools = WebTools::new(fetch);
        match tools
            .call("repo_read", serde_json::json!({"path": "ancho.txt"}))
            .await
        {
            ToolOutcome::Ok { content } => {
                assert!(content.contains("40 000"), "{content}");
            }
            other => panic!("la lectura debe capar caracteres: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_rejects_unsafe_and_unknown_paths() {
        let fetch = MockFetch::new();
        fetch.respond(
            GITHUB_API_TREE_URL,
            200,
            GITHUB_API_TREE_URL,
            &tree_body(&[("docs/a.md", Some(10))]),
        );
        let log = fetch.log();
        let tools = WebTools::new(fetch);
        for (path, hint) in [
            ("../x", ".."),
            ("/etc/passwd", "relativa"),
            ("C:\\x", "barras"),
            ("https://evil", "URLs"),
            ("docs/no-existe.md", "no existe"),
        ] {
            match tools
                .call("repo_read", serde_json::json!({"path": path}))
                .await
            {
                ToolOutcome::Failed { message } => {
                    assert!(
                        message.contains(hint),
                        "{path} debe explicar {hint:?}: {message}"
                    )
                }
                other => panic!("{path} debe rechazarse: {other:?}"),
            }
        }
        // Nothing rejected ever reached the raw host.
        assert!(
            log.lock()
                .expect("lock")
                .iter()
                .all(|url| !url.contains("raw.githubusercontent")),
            "lo rechazado no se descarga"
        );
    }

    #[tokio::test]
    async fn read_refuses_binary_and_oversized_files() {
        let fetch = MockFetch::new();
        fetch.respond(
            GITHUB_API_TREE_URL,
            200,
            GITHUB_API_TREE_URL,
            &tree_body(&[("bin.dat", Some(100)), ("gordo.bin", Some(2_000_000))]),
        );
        let bin_url = format!("{GITHUB_RAW_BASE}bin.dat");
        fetch.respond(&bin_url, 200, &bin_url, b"hola\0mundo");
        let log = fetch.log();
        let tools = WebTools::new(fetch);
        match tools
            .call("repo_read", serde_json::json!({"path": "bin.dat"}))
            .await
        {
            ToolOutcome::Failed { message } => {
                assert!(message.to_lowercase().contains("binario"), "{message}")
            }
            other => panic!("lo binario debe rechazarse: {other:?}"),
        }
        // Oversized: refused from the tree size, the raw file never travels.
        match tools
            .call("repo_read", serde_json::json!({"path": "gordo.bin"}))
            .await
        {
            ToolOutcome::Failed { message } => assert!(message.contains("grande"), "{message}"),
            other => panic!("lo gigante debe rechazarse: {other:?}"),
        }
        assert_eq!(count(&log, &format!("{GITHUB_RAW_BASE}gordo.bin")), 0);
    }

    #[tokio::test]
    async fn hlab_answer_is_readable_text_capped_and_cached() {
        let html = format!("<h1>EntropIA Lite</h1><p>{}</p>", "texto ".repeat(20_000));
        let fetch = MockFetch::new();
        fetch.respond(HLAB_PAGE_URL, 200, HLAB_PAGE_URL, html.as_bytes());
        let log = fetch.log();
        let tools = WebTools::new(fetch);
        let first = match tools
            .call("hlab_entropia_page", serde_json::json!({}))
            .await
        {
            ToolOutcome::Ok { content } => content,
            other => panic!("la página debe leerse: {other:?}"),
        };
        assert!(first.contains("EntropIA Lite"), "{first}");
        assert!(first.contains("30 000"), "{first}");
        let second = match tools
            .call("hlab_entropia_page", serde_json::json!({}))
            .await
        {
            ToolOutcome::Ok { content } => content,
            other => panic!("la página debe leerse: {other:?}"),
        };
        assert_eq!(first, second);
        assert_eq!(count(&log, HLAB_PAGE_URL), 1);
    }

    #[tokio::test]
    async fn allowlist_refuses_other_hosts_and_redirects() {
        let fetch = MockFetch::new();
        fetch.respond(GITHUB_API_TREE_URL, 200, "https://evil.example/robo", b"{}");
        let log = fetch.log();
        let tools = WebTools::new(fetch);
        // A redirect off the allowed hosts is refused, even with a 200.
        let error = tools
            .fetch_checked(GITHUB_API_TREE_URL)
            .await
            .expect_err("la redirección debe rechazarse");
        assert!(error.contains("permitid"), "{error}");
        // A crafted URL never reaches the fetcher.
        let error = tools
            .fetch_checked("https://evil.example/robo")
            .await
            .expect_err("el host ajeno debe rechazarse");
        assert!(error.contains("permitid"), "{error}");
        assert_eq!(log.lock().expect("lock").len(), 1, "lo prohibido no viaja");
    }

    #[tokio::test]
    async fn github_403_rate_limit_is_a_spanish_failure() {
        let fetch = MockFetch::new();
        fetch.respond(
            GITHUB_API_TREE_URL,
            403,
            GITHUB_API_TREE_URL,
            br#"{"message": "API rate limit exceeded for 1.2.3.4."}"#,
        );
        let tools = WebTools::new(fetch);
        match tools.call("repo_tree", serde_json::json!({})).await {
            ToolOutcome::Failed { message } => {
                assert!(message.contains("GitHub"), "{message}");
                assert!(message.to_lowercase().contains("límite"), "{message}");
            }
            other => panic!("el límite debe explicarse en español: {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_200_answers_fail_with_the_status() {
        let fetch = MockFetch::new();
        fetch.respond(
            GITHUB_API_TREE_URL,
            200,
            GITHUB_API_TREE_URL,
            &tree_body(&[("falta.txt", Some(10))]),
        );
        let url = format!("{GITHUB_RAW_BASE}falta.txt");
        fetch.respond(&url, 404, &url, b"no existe");
        let tools = WebTools::new(fetch);
        match tools
            .call("repo_read", serde_json::json!({"path": "falta.txt"}))
            .await
        {
            ToolOutcome::Failed { message } => assert!(message.contains("404"), "{message}"),
            other => panic!("el 404 debe fallar con su estado: {other:?}"),
        }
    }
}
