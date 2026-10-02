//! What a copy puts in Zotero, and how it compares with what is already there.
//! Pure: no network, no database.
//!
//! The four fields a copy owns are `title`, `url`, `accessDate` and
//! `websiteTitle`. The merge rule for an item that is already in Zotero:
//!
//! - a field empty in Zotero is **filled**;
//! - a field that differs and still holds what a previous copy wrote is
//!   **updated** (nobody touched it);
//! - a field that differs and is not what a previous copy wrote is **kept**:
//!   the person edited it in Zotero (or there is no record of what we wrote),
//!   and it is never overwritten.
//!
//! The connector can only create, so [`plan_merge`] is reported, not applied;
//! see the module docs of the parent.

use serde_json::{json, Value};
use tauri::Url;

/// What the saved source says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFacts {
    pub source_id: String,
    pub title: Option<String>,
    pub final_url: String,
    pub canonical_url: Option<String>,
    pub site_name: Option<String>,
    /// UTC, RFC 3339: the capture's, or the source's first access.
    pub accessed_at: String,
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn non_blank(text: Option<&str>) -> Option<String> {
    text.map(squash).filter(|text| !text.is_empty())
}

/// An address as stored in the item: no fragment.
fn item_url(address: &str) -> String {
    match Url::parse(address.trim()) {
        Ok(mut url) => {
            url.set_fragment(None);
            url.to_string()
        }
        Err(_) => address.trim().to_string(),
    }
}

/// The title of the item: the page title, else the host, else the address.
pub fn item_title(facts: &SourceFacts) -> String {
    if let Some(title) = non_blank(facts.title.as_deref()) {
        return title;
    }
    Url::parse(&facts.final_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| squash(&facts.final_url))
}

/// The Zotero connector item for a web page. `connector_id` names it inside
/// the save session so a PDF can be attached to it.
pub fn webpage_item(facts: &SourceFacts, connector_id: &str) -> Value {
    let mut item = json!({
        "id": connector_id,
        "itemType": "webpage",
        "title": item_title(facts),
        "url": item_url(&facts.final_url),
        "accessDate": normalize_instant(&facts.accessed_at),
    });
    if let Some(site) = non_blank(facts.site_name.as_deref()) {
        item["websiteTitle"] = json!(site);
    }
    item
}

/// Addresses compare without fragment, trailing slash on a path or host case.
pub fn normalize_url(address: &str) -> String {
    match Url::parse(address.trim()) {
        Ok(mut url) => {
            url.set_fragment(None);
            let path = url.path().to_string();
            if path.len() > 1 && path.ends_with('/') {
                url.set_path(path.trim_end_matches('/'));
            }
            url.to_string()
        }
        Err(_) => address.trim().to_string(),
    }
}

/// The addresses an existing item is looked up by: where the page ended up and
/// its canonical address, when that is another one.
pub fn lookup_urls(facts: &SourceFacts) -> Vec<String> {
    let mut urls = vec![item_url(&facts.final_url)];
    if let Some(canonical) = non_blank(facts.canonical_url.as_deref()) {
        let canonical = item_url(&canonical);
        if normalize_url(&canonical) != normalize_url(&urls[0]) {
            urls.push(canonical);
        }
    }
    urls
}

/// An instant to the second, in the spelling Zotero writes (`...T..:..:..Z`).
pub fn normalize_instant(text: &str) -> String {
    let trimmed = text.trim().replace(' ', "T");
    let head: String = trimmed.chars().take(19).collect();
    if head.len() == 19 {
        format!("{head}Z")
    } else {
        trimmed
    }
}

/// The fields a copy owns.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Owned {
    pub title: Option<String>,
    pub url: Option<String>,
    pub access_date: Option<String>,
    pub website_title: Option<String>,
}

impl Owned {
    /// The value of a field by its Zotero name.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields()
            .into_iter()
            .find(|(field, _)| *field == name)
            .and_then(|(_, value)| value.as_deref())
    }

    fn fields(&self) -> [(&'static str, &Option<String>); 4] {
        [
            ("title", &self.title),
            ("url", &self.url),
            ("accessDate", &self.access_date),
            ("websiteTitle", &self.website_title),
        ]
    }
}

/// The owned fields of a Zotero item's `data`.
pub fn owned_from_zotero(data: &Value) -> Owned {
    let read = |name: &str| non_blank(data.get(name).and_then(Value::as_str));
    Owned {
        title: read("title"),
        url: read("url"),
        access_date: read("accessDate"),
        website_title: read("websiteTitle"),
    }
}

fn same(field: &str, a: &str, b: &str) -> bool {
    match field {
        "url" => normalize_url(a) == normalize_url(b),
        "accessDate" => normalize_instant(a) == normalize_instant(b),
        _ => squash(a) == squash(b),
    }
}

/// What differs between our fields and the item's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergePlan {
    pub fill: Vec<&'static str>,
    pub update: Vec<&'static str>,
    pub kept: Vec<&'static str>,
}

impl MergePlan {
    pub fn is_empty(&self) -> bool {
        self.fill.is_empty() && self.update.is_empty() && self.kept.is_empty()
    }
}

/// Compares `ours` with `theirs` (the item in Zotero), given `written`, what a
/// previous copy wrote there. See the module docs for the rule.
pub fn plan_merge(ours: &Owned, theirs: &Owned, written: &Owned) -> MergePlan {
    let mut plan = MergePlan::default();
    for (((name, ours), (_, theirs)), (_, written)) in ours
        .fields()
        .into_iter()
        .zip(theirs.fields())
        .zip(written.fields())
    {
        let Some(ours) = ours else { continue };
        match theirs {
            None => plan.fill.push(name),
            Some(theirs) if same(name, ours, theirs) => {}
            Some(theirs) => match written {
                Some(written) if same(name, written, theirs) => plan.update.push(name),
                _ => plan.kept.push(name),
            },
        }
    }
    plan
}

/// One existing item that has one of our addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingItem {
    pub key: String,
    pub version: u64,
    pub item_type: String,
    pub date_added: String,
    pub owned: Owned,
}

/// The items among `hits` that are works (not attachments, notes or
/// annotations) with exactly one of `urls`, oldest first.
pub fn matching_items(hits: &[Value], urls: &[String]) -> Vec<ExistingItem> {
    let wanted: Vec<String> = urls.iter().map(|url| normalize_url(url)).collect();
    let mut found: Vec<ExistingItem> = hits
        .iter()
        .filter_map(|hit| {
            let data = hit.get("data")?;
            let item_type = data.get("itemType")?.as_str()?;
            if matches!(item_type, "attachment" | "note" | "annotation") {
                return None;
            }
            let url = data.get("url")?.as_str()?;
            if !wanted.contains(&normalize_url(url)) {
                return None;
            }
            Some(ExistingItem {
                key: hit.get("key")?.as_str()?.to_string(),
                version: hit.get("version").and_then(Value::as_u64).unwrap_or(0),
                item_type: item_type.to_string(),
                date_added: data
                    .get("dateAdded")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                owned: owned_from_zotero(data),
            })
        })
        .collect();
    found.sort_by(|a, b| a.date_added.cmp(&b.date_added).then(a.key.cmp(&b.key)));
    found
}

/// The title of the PDF child, named by the file's hash so the same file is
/// recognised wherever it was attached.
pub fn attachment_title(sha256: &str) -> String {
    let short: String = sha256.chars().take(8).collect();
    format!("Captura web {short}.pdf")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn facts() -> SourceFacts {
        SourceFacts {
            source_id: "src1".into(),
            title: Some("  A  page\ttitle ".into()),
            final_url: "https://www.example.org/a/b?x=1#frag".into(),
            canonical_url: Some("https://example.org/a/b".into()),
            site_name: Some("Example".into()),
            accessed_at: "2026-10-01T10:00:00.250Z".into(),
        }
    }

    #[test]
    fn the_webpage_item_carries_the_four_fields_we_own() {
        let item = webpage_item(&facts(), "entropia-1");
        assert_eq!(item["id"], "entropia-1");
        assert_eq!(item["itemType"], "webpage");
        assert_eq!(item["title"], "A page title");
        assert_eq!(item["url"], "https://www.example.org/a/b?x=1");
        assert_eq!(item["accessDate"], "2026-10-01T10:00:00Z");
        assert_eq!(item["websiteTitle"], "Example");
        assert!(
            item.get("creators").is_none(),
            "a web source names no author"
        );
        assert!(
            item.get("tags").is_none(),
            "nothing is added to the person's tags"
        );
    }

    #[test]
    fn a_source_without_title_or_site_still_makes_a_valid_item() {
        let mut bare = facts();
        bare.title = None;
        bare.site_name = Some("  ".into());
        let item = webpage_item(&bare, "e");
        assert_eq!(item["title"], "www.example.org");
        assert!(item.get("websiteTitle").is_none());
        bare.final_url = "not a url".into();
        assert_eq!(webpage_item(&bare, "e")["title"], "not a url");
    }

    #[test]
    fn urls_compare_without_fragment_trailing_slash_or_host_case() {
        assert_eq!(
            normalize_url("HTTPS://Example.org/a/b/#top"),
            normalize_url("https://example.org/a/b")
        );
        assert_ne!(
            normalize_url("https://example.org/a?x=1"),
            normalize_url("https://example.org/a?x=2")
        );
        assert_eq!(
            normalize_url("https://example.org/"),
            "https://example.org/"
        );
        assert_eq!(normalize_url("  weird  "), "weird");
    }

    #[test]
    fn the_lookup_addresses_are_the_final_and_the_canonical_one() {
        assert_eq!(
            lookup_urls(&facts()),
            vec![
                "https://www.example.org/a/b?x=1".to_string(),
                "https://example.org/a/b".to_string()
            ]
        );
        let mut same = facts();
        same.canonical_url = Some("https://www.example.org/a/b?x=1#other".into());
        assert_eq!(lookup_urls(&same).len(), 1);
    }

    #[test]
    fn instants_compare_to_the_second_in_any_iso_spelling() {
        assert_eq!(
            normalize_instant("2026-10-01T10:00:00.250Z"),
            normalize_instant("2026-10-01 10:00:00")
        );
        assert_eq!(
            normalize_instant("2026-10-01T10:00:00+00:00"),
            "2026-10-01T10:00:00Z"
        );
    }

    fn zotero_item(key: &str, kind: &str, url: &str, title: &str) -> serde_json::Value {
        json!({
            "key": key, "version": 7,
            "data": { "key": key, "itemType": kind, "title": title, "url": url,
                      "dateAdded": "2026-09-01T00:00:00Z" }
        })
    }

    #[test]
    fn existing_items_are_matched_by_exact_normalized_url_on_works_only() {
        let hits = vec![
            zotero_item("AAAAAAAA", "attachment", "https://example.org/a/b", "snap"),
            zotero_item("BBBBBBBB", "webpage", "https://example.org/other", "other"),
            zotero_item(
                "CCCCCCCC",
                "journalArticle",
                "https://Example.org/a/b/#x",
                "Paper",
            ),
            zotero_item("DDDDDDDD", "note", "https://example.org/a/b", "n"),
        ];
        let found = matching_items(&hits, &lookup_urls(&facts()));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].key, "CCCCCCCC");
        assert_eq!(found[0].version, 7);
        assert_eq!(found[0].item_type, "journalArticle");
    }

    #[test]
    fn the_oldest_match_wins_when_there_are_several() {
        let mut old = zotero_item("OLDOLDOL", "webpage", "https://example.org/a/b", "x");
        old["data"]["dateAdded"] = json!("2020-01-01T00:00:00Z");
        let new = zotero_item("NEWNEWNE", "webpage", "https://example.org/a/b", "x");
        let found = matching_items(&[new, old], &lookup_urls(&facts()));
        assert_eq!(found[0].key, "OLDOLDOL");
    }

    fn owned(title: &str, url: &str, date: &str, site: &str) -> Owned {
        Owned {
            title: some(title),
            url: some(url),
            access_date: some(date),
            website_title: some(site),
        }
    }

    fn some(text: &str) -> Option<String> {
        (!text.is_empty()).then(|| text.to_string())
    }

    #[test]
    fn merge_fills_what_is_empty_and_leaves_what_agrees() {
        let ours = owned("T", "https://a.test", "2026-10-01T10:00:00Z", "Site");
        let theirs = owned("T", "https://a.test", "", "");
        let merge = plan_merge(&ours, &theirs, &Owned::default());
        assert_eq!(merge.fill, vec!["accessDate", "websiteTitle"]);
        assert!(merge.update.is_empty() && merge.kept.is_empty());
        assert!(!merge.is_empty());
    }

    #[test]
    fn merge_updates_only_what_we_wrote_and_nobody_edited() {
        let ours = owned(
            "New title",
            "https://a.test",
            "2026-10-01T10:00:00Z",
            "Site",
        );
        let written = owned(
            "Old title",
            "https://a.test",
            "2026-10-01T10:00:00Z",
            "Site",
        );
        let untouched = owned(
            "Old title",
            "https://a.test",
            "2026-10-01T10:00:00Z",
            "Site",
        );
        let merge = plan_merge(&ours, &untouched, &written);
        assert_eq!(merge.update, vec!["title"]);
        assert!(merge.kept.is_empty());
    }

    #[test]
    fn merge_never_touches_a_field_the_person_edited() {
        let ours = owned(
            "New title",
            "https://a.test",
            "2026-10-01T10:00:00Z",
            "Site",
        );
        let written = owned(
            "Old title",
            "https://a.test",
            "2026-10-01T10:00:00Z",
            "Site",
        );
        let edited = owned(
            "My own title",
            "https://a.test",
            "2026-10-01T10:00:00Z",
            "Site",
        );
        let merge = plan_merge(&ours, &edited, &written);
        assert!(merge.update.is_empty());
        assert_eq!(merge.kept, vec!["title"]);
    }

    #[test]
    fn without_a_record_of_what_we_wrote_a_difference_is_kept_not_updated() {
        let ours = owned("Ours", "https://a.test", "2026-10-01T10:00:00Z", "Site");
        let theirs = owned("Theirs", "https://a.test", "2026-10-01T10:00:00Z", "Site");
        let merge = plan_merge(&ours, &theirs, &Owned::default());
        assert_eq!(merge.kept, vec!["title"]);
        assert!(merge.update.is_empty());
    }

    #[test]
    fn dates_that_differ_only_in_spelling_are_the_same() {
        let ours = owned("T", "https://a.test", "2026-10-01T10:00:00.5Z", "S");
        let theirs = owned("T", "https://a.test/", "2026-10-01 10:00:00", "S");
        assert!(plan_merge(&ours, &theirs, &Owned::default()).is_empty());
    }

    #[test]
    fn zotero_data_is_read_into_the_owned_fields() {
        let data = json!({ "title": "T", "url": "u", "accessDate": "d", "websiteTitle": " " });
        let read = owned_from_zotero(&data);
        assert_eq!(read.title.as_deref(), Some("T"));
        assert_eq!(read.website_title, None);
    }

    #[test]
    fn the_attachment_title_names_the_file_by_its_hash() {
        assert_eq!(
            attachment_title(&"ab".repeat(32)),
            "Captura web abababab.pdf"
        );
        assert_eq!(attachment_title("xy"), "Captura web xy.pdf");
    }
}
