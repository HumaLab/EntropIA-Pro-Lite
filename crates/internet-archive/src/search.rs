//! Advanced Search: query model with Lucene escaping, endpoints, result
//! types, and tolerant response parsing.
//!
//! Every user-supplied value is treated as a literal: it is wrapped in
//! double quotes with `\` and `"` escaped, so operators (`AND`, `OR`,
//! `NOT`), field prefixes (`field:`), wildcards (`*`, `?`), grouping and
//! range syntax typed by the user end up inside quotes as plain text.

use serde_json::Value;
use url::Url;

use crate::error::IaError;

/// Where to send API requests.
#[derive(Debug, Clone)]
pub struct Endpoints {
    base: Url,
}

impl Endpoints {
    /// Production endpoints: `https://archive.org`.
    pub fn production() -> Self {
        Self {
            base: Url::parse("https://archive.org").expect("production base URL must parse"),
        }
    }

    /// Custom base URL; tests point this at the local server.
    pub fn new(base: Url) -> Self {
        Self { base }
    }

    /// The base URL requests are built from.
    pub fn base(&self) -> &Url {
        &self.base
    }
}

/// Structured Advanced Search query. Optional filters combine with `AND`;
/// `texts_only` (on by default) adds `mediatype:texts`.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// Free text: split on whitespace, each term a quoted literal.
    pub text: Option<String>,
    /// Literal matched against the `title` field.
    pub title: Option<String>,
    /// Literal matched against the `creator` field.
    pub creator: Option<String>,
    /// Literal matched against the `language` field.
    pub language: Option<String>,
    /// Collection identifier, `^[A-Za-z0-9._-]{1,100}$`.
    pub collection: Option<String>,
    /// Inclusive lower year bound, rendered into a `date:[A TO B]` range.
    pub year_from: Option<u16>,
    /// Inclusive upper year bound, rendered into a `date:[A TO B]` range.
    pub year_to: Option<u16>,
    /// When true (default), the query also requires `mediatype:texts`.
    pub texts_only: bool,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            text: None,
            title: None,
            creator: None,
            language: None,
            collection: None,
            year_from: None,
            year_to: None,
            texts_only: true,
        }
    }
}

impl SearchQuery {
    /// Render the query as a Lucene string for the `q` parameter.
    ///
    /// Returns [`IaError::InvalidQuery`] when a filter is malformed
    /// (bad collection, bad year range) or when no user clause remains:
    /// an unfiltered search is never sent.
    pub fn to_lucene(&self) -> Result<String, IaError> {
        let mut clauses: Vec<String> = Vec::new();
        if let Some(text) = &self.text {
            for term in text.split_whitespace() {
                clauses.push(quote(term));
            }
        }
        if let Some(title) = trimmed(&self.title) {
            clauses.push(format!("title:({})", quote(title)));
        }
        if let Some(creator) = trimmed(&self.creator) {
            clauses.push(format!("creator:({})", quote(creator)));
        }
        if let Some(language) = trimmed(&self.language) {
            clauses.push(format!("language:({})", quote(language)));
        }
        if let Some(collection) = &self.collection {
            check_collection(collection)?;
            clauses.push(format!("collection:({})", quote(collection)));
        }
        if let Some(date) = date_clause(self.year_from, self.year_to)? {
            clauses.push(date);
        }
        if clauses.is_empty() {
            return Err(IaError::InvalidQuery {
                reason: "query has no search clause; \
                         refusing to send an unfiltered search"
                    .to_string(),
            });
        }
        if self.texts_only {
            clauses.push("mediatype:texts".to_string());
        }
        Ok(clauses.join(" AND "))
    }
}

/// Quote one literal: escape `\` and `"` so the value can never act as
/// Lucene syntax. Everything else stays inside the quotes as plain text.
fn quote(literal: &str) -> String {
    let escaped = literal.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Trimmed value, or `None` when absent or whitespace-only. Whitespace-only
/// filters do not count as a search clause.
fn trimmed(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Validate a collection identifier against `^[A-Za-z0-9._-]{1,100}$`.
fn check_collection(collection: &str) -> Result<(), IaError> {
    let ok = !collection.is_empty()
        && collection.len() <= 100
        && collection
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(IaError::InvalidQuery {
            reason: format!("invalid collection identifier: {collection:?}"),
        })
    }
}

/// Render the year range as `date:[YYYY-01-01 TO YYYY-12-31]` (zero-padded)
/// with either bound open (`*`) when missing.
fn date_clause(from: Option<u16>, to: Option<u16>) -> Result<Option<String>, IaError> {
    for (name, year) in [("year_from", from), ("year_to", to)] {
        if let Some(y) = year {
            if y > 9999 {
                return Err(IaError::InvalidQuery {
                    reason: format!("{name} {y} out of range 0..=9999"),
                });
            }
        }
    }
    if let (Some(f), Some(t)) = (from, to) {
        if f > t {
            return Err(IaError::InvalidQuery {
                reason: format!("year_from {f} is after year_to {t}"),
            });
        }
    }
    match (from, to) {
        (None, None) => Ok(None),
        (Some(f), Some(t)) => Ok(Some(format!("date:[{f:04}-01-01 TO {t:04}-12-31]"))),
        (Some(f), None) => Ok(Some(format!("date:[{f:04}-01-01 TO *]"))),
        (None, Some(t)) => Ok(Some(format!("date:[* TO {t:04}-12-31]"))),
    }
}

/// One normalized Advanced Search hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchItem {
    /// Item identifier (documents without one are skipped).
    pub identifier: String,
    /// First title string, if any.
    pub title: Option<String>,
    /// Normalized `creator` values.
    pub creators: Vec<String>,
    /// First `date` string, if any (source text preserved).
    pub date: Option<String>,
    /// Normalized `collection` values.
    pub collections: Vec<String>,
    /// True when `access-restricted-item` is `"true"`, `true`, `1` or `"1"`.
    pub restricted: bool,
}

/// One page of Advanced Search results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPage {
    /// Total hits (`response.numFound`, number or numeric string).
    pub num_found: u64,
    /// Normalized documents on this page.
    pub items: Vec<SearchItem>,
}

/// Build a [`SearchPage`] from a raw response value. Never fails on field
/// shapes: scalars accept string/number/bool, lists accept a scalar or an
/// array, missing fields become `None`/empty, and documents without an
/// identifier are skipped.
pub(crate) fn parse_search_value(body: &Value) -> SearchPage {
    let response = body.get("response");
    let num_found = num_found_in(body);
    let mut items = Vec::new();
    if let Some(docs) = response
        .and_then(|r| r.get("docs"))
        .and_then(Value::as_array)
    {
        for doc in docs {
            let Some(obj) = doc.as_object() else {
                continue;
            };
            let Some(identifier) = identifier_in(obj) else {
                continue;
            };
            items.push(SearchItem {
                identifier,
                title: obj.get("title").and_then(first_string),
                creators: string_list(obj.get("creator").or_else(|| obj.get("creators"))),
                date: obj.get("date").and_then(first_string),
                collections: string_list(obj.get("collection").or_else(|| obj.get("collections"))),
                restricted: restricted_in(obj),
            });
        }
    }
    SearchPage { num_found, items }
}

/// Scalar to string: strings as-is (empty becomes `None`), numbers and
/// bools stringified, everything else `None`.
fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => {
            if s.is_empty() {
                None
            } else {
                Some(s.clone())
            }
        }
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// First usable string for scalar fields (`title`, `date`): a bare scalar
/// stringified, or the first convertible element of an array.
fn first_string(value: &Value) -> Option<String> {
    match value {
        Value::Array(items) => items.iter().filter_map(scalar_string).next(),
        _ => scalar_string(value),
    }
}

/// String list for `creator`/`collection`: a bare scalar becomes one entry,
/// an array contributes every convertible entry, missing/null is empty.
fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.iter().filter_map(scalar_string).collect(),
        Some(single) => scalar_string(single).into_iter().collect(),
    }
}

/// Usable identifier: non-empty string (trimmed), number/bool stringified,
/// or the first convertible array element; otherwise `None` (doc skipped).
fn identifier_in(doc: &serde_json::Map<String, Value>) -> Option<String> {
    match doc.get("identifier") {
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => {
                    let trimmed = s.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                }
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => None,
            })
            .next(),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    }
}

/// True for `"true"` (any case, padded), `true`, `1` and `"1"`; an array
/// is true when any element matches.
fn restricted_in(doc: &serde_json::Map<String, Value>) -> bool {
    match doc.get("access-restricted-item") {
        None | Some(Value::Null) => false,
        Some(Value::Array(items)) => items.iter().any(scalar_is_truthy),
        Some(single) => scalar_is_truthy(single),
    }
}

fn scalar_is_truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => {
            n.as_u64() == Some(1) || n.as_i64() == Some(1) || n.as_f64() == Some(1.0)
        }
        Value::String(s) => {
            let text = s.trim().to_ascii_lowercase();
            text == "true" || text == "1"
        }
        _ => false,
    }
}

/// `numFound` arrives as a number or a numeric string; anything else
/// (missing, null, garbage) is 0 rather than a page failure.
fn num_found_in(body: &Value) -> u64 {
    match body.get("response").and_then(|r| r.get("numFound")) {
        Some(Value::Number(n)) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|i| u64::try_from(i).ok()))
            .or_else(|| {
                n.as_f64().and_then(|f| {
                    if f >= 0.0 && f.fract() == 0.0 {
                        Some(f as u64)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse::<u64>().unwrap_or(0),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use crate::policy::FetchPolicy;
    use crate::test_support::TestServer;
    use crate::{Endpoints, IaClient, IaError, SearchQuery};
    use url::Url;

    const FIXTURE: &str = include_str!("../tests/fixtures/advancedsearch-mar-del-plata.json");

    fn query(fields: &[(&str, &str)]) -> SearchQuery {
        let mut q = SearchQuery::default();
        for (k, v) in fields {
            match *k {
                "text" => q.text = Some((*v).to_string()),
                "title" => q.title = Some((*v).to_string()),
                "creator" => q.creator = Some((*v).to_string()),
                "language" => q.language = Some((*v).to_string()),
                "collection" => q.collection = Some((*v).to_string()),
                _ => panic!("unknown field {k}"),
            }
        }
        q
    }

    fn local_endpoints(server: &TestServer) -> Endpoints {
        Endpoints::new(Url::parse(server.base()).unwrap())
    }

    fn local_client() -> IaClient {
        IaClient::new(FetchPolicy::loopback_for_tests())
    }

    // ---- clause builder ----

    #[test]
    fn builder_plain_text_splits_into_quoted_terms() {
        let lucene = query(&[("text", "Mar del Plata")]).to_lucene().unwrap();
        assert_eq!(
            lucene,
            "\"Mar\" AND \"del\" AND \"Plata\" AND mediatype:texts"
        );
    }

    #[test]
    fn builder_text_syntax_chars_stay_literal() {
        // `:` prefixes, parens, wildcards and operators must end up inside
        // quotes so they never act as Lucene syntax.
        let lucene = query(&[("text", "title:foo (bar) * ?")])
            .to_lucene()
            .unwrap();
        assert_eq!(
            lucene,
            "\"title:foo\" AND \"(bar)\" AND \"*\" AND \"?\" AND mediatype:texts"
        );
        let lucene = query(&[("text", "a OR b NOT c AND d")])
            .to_lucene()
            .unwrap();
        assert_eq!(
            lucene,
            "\"a\" AND \"OR\" AND \"b\" AND \"NOT\" AND \"c\" AND \"AND\" AND \"d\" \
             AND mediatype:texts"
        );
        let lucene = query(&[("text", "a && b || c ~d e^2 [f]")])
            .to_lucene()
            .unwrap();
        assert_eq!(
            lucene,
            "\"a\" AND \"&&\" AND \"b\" AND \"||\" AND \"c\" AND \"~d\" AND \"e^2\" \
             AND \"[f]\" AND mediatype:texts"
        );
    }

    #[test]
    fn builder_escapes_backslash_and_quote() {
        let lucene = query(&[("text", "a\"b\\c")]).to_lucene().unwrap();
        assert_eq!(lucene, "\"a\\\"b\\\\c\" AND mediatype:texts");
        let lucene = query(&[("title", "A \"quoted\" (title): *?")])
            .to_lucene()
            .unwrap();
        assert_eq!(
            lucene,
            "title:(\"A \\\"quoted\\\" (title): *?\") AND mediatype:texts"
        );
    }

    #[test]
    fn builder_field_values_stay_literal() {
        let lucene = query(&[("title", "Mar del Plata: (edición)* OR NOT")])
            .to_lucene()
            .unwrap();
        assert_eq!(
            lucene,
            "title:(\"Mar del Plata: (edición)* OR NOT\") AND mediatype:texts"
        );
        let lucene = query(&[
            ("text", "x"),
            ("title", "t"),
            ("creator", "c"),
            ("language", "eng"),
            ("collection", "opensource"),
        ])
        .to_lucene()
        .unwrap();
        assert_eq!(
            lucene,
            "\"x\" AND title:(\"t\") AND creator:(\"c\") AND language:(\"eng\") \
             AND collection:(\"opensource\") AND mediatype:texts"
        );
    }

    #[test]
    fn builder_collection_valid() {
        for id in ["opensource", "a.b_c-d", "A9", "x"] {
            let lucene = query(&[("collection", id)]).to_lucene().unwrap();
            assert_eq!(lucene, format!("collection:(\"{id}\") AND mediatype:texts"));
        }
    }

    #[test]
    fn builder_collection_invalid() {
        let long = "x".repeat(101);
        for id in [
            "has space",
            "semi;colon",
            "a/b",
            "a*b",
            "evil!",
            "",
            long.as_str(),
        ] {
            let res = query(&[("collection", id)]).to_lucene();
            assert!(
                matches!(res, Err(IaError::InvalidQuery { .. })),
                "{id:?} should be rejected, got {res:?}"
            );
        }
    }

    #[test]
    fn builder_years_valid() {
        let mut q = query(&[("collection", "opensource")]);
        q.year_from = Some(1990);
        q.year_to = Some(2000);
        assert_eq!(
            q.to_lucene().unwrap(),
            "collection:(\"opensource\") AND date:[1990-01-01 TO 2000-12-31] \
             AND mediatype:texts"
        );
    }

    #[test]
    fn builder_years_open_ended_and_padded() {
        let mut q = query(&[("text", "x")]);
        q.year_from = Some(2001);
        assert_eq!(
            q.to_lucene().unwrap(),
            "\"x\" AND date:[2001-01-01 TO *] AND mediatype:texts"
        );
        let mut q = query(&[("text", "x")]);
        q.year_to = Some(1999);
        assert_eq!(
            q.to_lucene().unwrap(),
            "\"x\" AND date:[* TO 1999-12-31] AND mediatype:texts"
        );
        // A bare year range is a user clause on its own.
        let q = SearchQuery {
            year_from: Some(5),
            year_to: Some(7),
            ..SearchQuery::default()
        };
        assert_eq!(
            q.to_lucene().unwrap(),
            "date:[0005-01-01 TO 0007-12-31] AND mediatype:texts"
        );
    }

    #[test]
    fn builder_years_invalid() {
        // Inverted range.
        let mut q = query(&[("text", "x")]);
        q.year_from = Some(2000);
        q.year_to = Some(1990);
        assert!(matches!(q.to_lucene(), Err(IaError::InvalidQuery { .. })));
        // Out of range (> 9999).
        let mut q = query(&[("text", "x")]);
        q.year_from = Some(10_000);
        assert!(matches!(q.to_lucene(), Err(IaError::InvalidQuery { .. })));
        let mut q = query(&[("text", "x")]);
        q.year_to = Some(65_535);
        assert!(matches!(q.to_lucene(), Err(IaError::InvalidQuery { .. })));
    }

    #[test]
    fn builder_empty_query_rejected() {
        assert!(matches!(
            SearchQuery::default().to_lucene(),
            Err(IaError::InvalidQuery { .. })
        ));
        let q = SearchQuery {
            texts_only: false,
            ..SearchQuery::default()
        };
        assert!(matches!(q.to_lucene(), Err(IaError::InvalidQuery { .. })));
        // Whitespace-only values do not count as a clause either.
        assert!(matches!(
            query(&[("text", "   ")]).to_lucene(),
            Err(IaError::InvalidQuery { .. })
        ));
    }

    #[test]
    fn builder_texts_only_false() {
        let mut q = query(&[("text", "x")]);
        q.texts_only = false;
        assert_eq!(q.to_lucene().unwrap(), "\"x\"");
        let mut q = query(&[("title", "Mar del Plata")]);
        q.texts_only = false;
        assert_eq!(q.to_lucene().unwrap(), "title:(\"Mar del Plata\")");
    }

    // ---- request shape ----

    fn last_target(server: &TestServer) -> Url {
        let targets = server.targets();
        let target = targets.last().expect("server should have seen a request");
        Url::parse(&format!("http://127.0.0.1{target}")).unwrap()
    }

    #[tokio::test]
    async fn request_shape_exact() {
        let server = TestServer::start(vec![TestServer::json(
            "/advancedsearch.php",
            200,
            r#"{"response":{"numFound":0,"docs":[]}}"#,
        )]);
        let endpoints = local_endpoints(&server);
        let page = local_client()
            .search(&endpoints, &query(&[("title", "Mar del Plata")]), 1, 50)
            .await
            .unwrap();
        assert_eq!(page.num_found, 0);
        assert!(page.items.is_empty());

        let url = last_target(&server);
        assert_eq!(url.path(), "/advancedsearch.php");
        let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let get = |k: &str| {
            pairs
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(
            get("q").as_deref(),
            Some("title:(\"Mar del Plata\") AND mediatype:texts")
        );
        let fls: Vec<String> = pairs
            .iter()
            .filter(|(k, _)| k == "fl[]")
            .map(|(_, v)| v.clone())
            .collect();
        assert_eq!(
            fls,
            [
                "identifier",
                "title",
                "creator",
                "date",
                "collection",
                "access-restricted-item"
            ]
        );
        assert_eq!(get("rows").as_deref(), Some("50"));
        assert_eq!(get("page").as_deref(), Some("1"));
        assert_eq!(get("output").as_deref(), Some("json"));
    }

    #[tokio::test]
    async fn request_rows_and_page_clamped() {
        let server = TestServer::start(vec![TestServer::json(
            "/advancedsearch.php",
            200,
            r#"{"response":{"numFound":0,"docs":[]}}"#,
        )]);
        let endpoints = local_endpoints(&server);
        local_client()
            .search(&endpoints, &query(&[("text", "x")]), 0, 500)
            .await
            .unwrap();
        let url = last_target(&server);
        let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let get = |k: &str| {
            pairs
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("rows").as_deref(), Some("100"));
        assert_eq!(get("page").as_deref(), Some("1"));
    }

    #[tokio::test]
    async fn deep_paging_rejected_without_request() {
        let server = TestServer::start(vec![TestServer::json(
            "/advancedsearch.php",
            200,
            r#"{"response":{"numFound":0,"docs":[]}}"#,
        )]);
        let endpoints = local_endpoints(&server);
        let res = local_client()
            .search(&endpoints, &query(&[("text", "x")]), 101, 100)
            .await;
        assert!(
            matches!(res, Err(IaError::InvalidQuery { .. })),
            "got {res:?}"
        );
        assert!(
            server.targets().is_empty(),
            "no request must be sent, saw {:?}",
            server.targets()
        );
        // The boundary itself is still allowed.
        local_client()
            .search(&endpoints, &query(&[("text", "x")]), 100, 100)
            .await
            .unwrap();
        assert_eq!(server.targets().len(), 1);
    }

    // ---- parsing ----

    #[tokio::test]
    async fn parses_real_fixture() {
        let server = TestServer::start(vec![TestServer::json("/advancedsearch.php", 200, FIXTURE)]);
        let endpoints = local_endpoints(&server);
        let page = local_client()
            .search(&endpoints, &query(&[("title", "Mar del Plata")]), 1, 3)
            .await
            .unwrap();
        assert_eq!(page.num_found, 283);
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.items[0].identifier, "proceedingsofvic00cong");
        assert!(page.items[0].restricted);
        assert_eq!(
            page.items[0].title.as_deref(),
            Some(
                "Proceedings of the VI Congreso Latinoamericano de Botánica, \
                 Mar del Plata, Argentina, 2-8 October 1994"
            )
        );
        assert!(!page.items[1].restricted);
        assert!(!page.items[2].restricted);
        assert_eq!(
            page.items[1].identifier,
            "30-festival-notas-para-una-contra-historia-del-cine-argentino"
        );
        assert_eq!(page.items[2].identifier, "harfr.apb.mardelplata");
    }

    #[tokio::test]
    async fn tolerant_parsing_shapes() {
        let server = TestServer::start(vec![TestServer::json(
            "/advancedsearch.php",
            200,
            r#"{
                "response": {
                    "numFound": "2",
                    "docs": [
                        {"identifier": "a", "creator": "Solo"},
                        {"identifier": "b", "creator": ["One", "Two"],
                         "title": ["First", "Second"], "date": 1994,
                         "collection": "opensource",
                         "access-restricted-item": 1},
                        {"title": "no identifier here"},
                        {"identifier": ""},
                        {"identifier": "c", "access-restricted-item": "false"}
                    ]
                }
            }"#,
        )]);
        let endpoints = local_endpoints(&server);
        let page = local_client()
            .search(&endpoints, &query(&[("text", "x")]), 1, 50)
            .await
            .unwrap();
        // Numeric numFound arrives as a string; docs without a usable
        // identifier are skipped instead of failing the page.
        assert_eq!(page.num_found, 2);
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.items[0].creators, vec!["Solo"]);
        assert!(page.items[0].title.is_none());
        assert!(!page.items[0].restricted);
        assert_eq!(page.items[1].creators, vec!["One", "Two"]);
        assert_eq!(page.items[1].title.as_deref(), Some("First"));
        assert_eq!(page.items[1].date.as_deref(), Some("1994"));
        assert_eq!(page.items[1].collections, vec!["opensource"]);
        assert!(page.items[1].restricted);
        assert_eq!(page.items[2].identifier, "c");
        assert!(!page.items[2].restricted);
    }

    #[tokio::test]
    async fn restricted_flag_variants() {
        for (raw, expected) in [
            (r#""true""#, true),
            (r#"true"#, true),
            (r#"1"#, true),
            (r#""1""#, true),
            (r#""false""#, false),
            (r#"false"#, false),
            (r#"0"#, false),
            (r#""0""#, false),
        ] {
            let body = format!(
                "{{\"response\":{{\"numFound\":1,\"docs\":[{{\"identifier\":\"i\",\
                  \"access-restricted-item\":{raw}}}]}}}}"
            );
            let server =
                TestServer::start(vec![TestServer::json("/advancedsearch.php", 200, &body)]);
            let endpoints = local_endpoints(&server);
            let page = local_client()
                .search(&endpoints, &query(&[("text", "x")]), 1, 10)
                .await
                .unwrap();
            assert_eq!(page.items.len(), 1);
            assert_eq!(page.items[0].restricted, expected, "raw={raw}");
        }
    }
}
