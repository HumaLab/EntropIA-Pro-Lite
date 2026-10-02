//! The browser's tabs, the pure part: how their webviews are named, how many
//! may exist, which new-window requests become a tab and which a popup window,
//! and the list that keeps them in order.
//!
//! Each tab is its own child webview of the main window, labelled
//! `navegador-web-<n>`. Like the popup windows (see [`super::popup`]) it has no
//! capability at all: the ACL refuses every command and plugin call from it
//! (`tests/app_acl.rs`). This file has no dependency on the rest of the crate so
//! that test can include it and build its labels from the same code the app
//! uses.

use serde::Serialize;

/// Every tab label starts with this.
pub const LABEL_PREFIX: &str = "navegador-web-";

/// Tabs one browser may have open. It is the browser's own limit, unrelated to
/// how many workspace tabs the app allows.
pub const MAX_TABS: usize = 4;

/// Shown when a page asks for a tab and the limit is reached.
pub const TOO_MANY_MESSAGE: &str = "The page tried to open too many tabs";

/// Shown when the person asks for a tab and the limit is reached.
pub const LIMIT_MESSAGE: &str = "The browser already has the maximum number of tabs";

/// The label of tab `id`. Ids are never reused within a session, so two
/// webviews never share a label even while one is still closing.
pub fn label(id: u32) -> String {
    format!("{LABEL_PREFIX}{id}")
}

/// The id in `label`, if `label` is exactly one [`label`] could have made: the
/// prefix and a plain number, nothing else (`navegador-web-1`, not
/// `navegador-web-x`, `navegador-web-01` or `navegador-web-1-evil`).
pub fn tab_id(label: &str) -> Option<u32> {
    let rest = label.strip_prefix(LABEL_PREFIX)?;
    let id: u32 = rest.parse().ok()?;
    (self::label(id) == label).then_some(id)
}

/// Whether `label` belongs to a tab.
pub fn is_tab_label(label: &str) -> bool {
    tab_id(label).is_some()
}

/// Whether one more tab may open when `open` already exist.
pub fn has_room(open: usize) -> bool {
    open < MAX_TABS
}

/// Where a page's request for a new window goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// A new tab in the browser (a plain link, `window.open(url)`).
    Tab,
    /// An isolated popup window that keeps `window.opener` (a sign-in flow).
    Popup,
}

/// A request that comes with a size or a position is a script opening a window
/// to talk to it (Google Sign-In asks for `width=...,height=...`): it needs the
/// opener link only a popup window keeps. Anything else is a plain link.
///
/// `features_reported` says whether the engine tells size and position at all.
/// WebView2 and WKWebView report them only when the page gave them; wry's
/// WebKitGTK never does, so there nothing tells a link from a sign-in window
/// and the safe answer is the popup, which keeps every flow working.
pub fn placement(features_reported: bool, has_size: bool, has_position: bool) -> Placement {
    if !features_reported || has_size || has_position {
        Placement::Popup
    } else {
        Placement::Tab
    }
}

/// One tab, as the main UI sees it. `url` is `None` until the tab has a page
/// (a blank tab has no webview yet).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
    pub id: u32,
    pub url: Option<String>,
    pub title: Option<String>,
    /// Why the last navigation of this tab was refused, until one works.
    pub blocked: Option<String>,
}

impl Tab {
    fn blank(id: u32) -> Self {
        Self {
            id,
            url: None,
            title: None,
            blocked: None,
        }
    }
}

/// What the frontend shows: every tab, in order, and which one is on screen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BrowserState {
    pub tabs: Vec<Tab>,
    pub active: Option<u32>,
    /// Grows with every snapshot. A command's answer and an event can reach the
    /// UI in either order; the one with the higher revision is the newer.
    pub revision: u64,
}

/// The limit was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full;

/// The tabs in the order the strip shows them, and which one is active. Once it
/// has a tab it always has one: closing the last tab leaves a fresh blank one,
/// so the browser is never without an address bar to type into.
#[derive(Debug, Default)]
pub struct TabList {
    tabs: Vec<Tab>,
    active: Option<u32>,
    last_id: u32,
    revision: u64,
}

impl TabList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active(&self) -> Option<u32> {
        self.active
    }

    pub fn ids(&self) -> Vec<u32> {
        self.tabs.iter().map(|tab| tab.id).collect()
    }

    pub fn get(&self, id: u32) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    /// Add a blank tab at the end. It becomes the active one when `activate`
    /// says so, or when there was none.
    pub fn add(&mut self, activate: bool) -> Result<u32, Full> {
        if !has_room(self.tabs.len()) {
            return Err(Full);
        }
        self.last_id += 1;
        let id = self.last_id;
        self.tabs.push(Tab::blank(id));
        if activate || self.active.is_none() {
            self.active = Some(id);
        }
        Ok(id)
    }

    /// Make `id` the active tab. False when there is no such tab.
    pub fn activate(&mut self, id: u32) -> bool {
        if self.get(id).is_none() {
            return false;
        }
        self.active = Some(id);
        true
    }

    /// Remove tab `id`. The tab that takes its place on screen is its right
    /// neighbour, or the left one when it was the last. The only tab left is
    /// replaced by a blank one instead of removed. False when there is no such
    /// tab.
    pub fn close(&mut self, id: u32) -> bool {
        let Some(at) = self.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        self.tabs.remove(at);
        if self.tabs.is_empty() {
            self.last_id += 1;
            let fresh = self.last_id;
            self.tabs.push(Tab::blank(fresh));
            self.active = Some(fresh);
        } else if self.active == Some(id) {
            let next = at.min(self.tabs.len() - 1);
            self.active = Some(self.tabs[next].id);
        }
        true
    }

    /// Forget every tab. Ids keep counting up, so a label is not reused.
    pub fn clear(&mut self) {
        self.tabs.clear();
        self.active = None;
    }

    /// The state as the UI sees it, stamped newer than every one before.
    pub fn snapshot(&mut self) -> BrowserState {
        self.revision += 1;
        BrowserState {
            tabs: self.tabs.clone(),
            active: self.active,
            revision: self.revision,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- labels -------------------------------------------------------------

    #[test]
    fn labels_carry_the_prefix_and_a_number() {
        assert_eq!(label(1), "navegador-web-1");
        assert_eq!(label(42), "navegador-web-42");
        assert!(label(7).starts_with(LABEL_PREFIX));
        assert_ne!(label(1), label(2));
    }

    #[test]
    fn a_label_made_here_gives_its_id_back() {
        for id in [0, 1, 9, 10, 4_000_000_000, u32::MAX] {
            assert_eq!(tab_id(&label(id)), Some(id), "{id}");
            assert!(is_tab_label(&label(id)), "{id}");
        }
    }

    #[test]
    fn only_that_shape_is_a_tab_label() {
        for other in [
            "main",
            "navegador-web",
            "navegador-web-",
            "navegador-web-x",
            "navegador-web-1-evil",
            "navegador-web-1 ",
            " navegador-web-1",
            "navegador-web--1",
            "navegador-web-+1",
            "navegador-web-01",
            "navegador-web-4294967296",
            "Navegador-web-1",
            "xnavegador-web-1",
            "navegador-popup-1",
            "",
        ] {
            assert_eq!(tab_id(other), None, "{other:?}");
            assert!(!is_tab_label(other), "{other:?}");
        }
    }

    // --- the limit ----------------------------------------------------------

    #[test]
    fn a_browser_gets_four_tabs_and_no_more() {
        assert_eq!(MAX_TABS, 4);
        assert!(has_room(0));
        assert!(has_room(MAX_TABS - 1));
        assert!(!has_room(MAX_TABS));
        assert!(!has_room(MAX_TABS + 1));
    }

    // --- tab or popup -------------------------------------------------------

    #[test]
    fn a_request_without_features_is_a_tab() {
        assert_eq!(placement(true, false, false), Placement::Tab);
    }

    #[test]
    fn a_request_with_a_size_or_a_position_is_a_popup_window() {
        assert_eq!(placement(true, true, false), Placement::Popup);
        assert_eq!(placement(true, false, true), Placement::Popup);
        assert_eq!(placement(true, true, true), Placement::Popup);
    }

    #[test]
    fn an_engine_that_reports_no_features_gets_the_popup_that_keeps_flows_working() {
        for (size, position) in [(false, false), (true, false), (false, true), (true, true)] {
            assert_eq!(placement(false, size, position), Placement::Popup);
        }
    }

    // --- the list -----------------------------------------------------------

    fn list_of(n: usize) -> TabList {
        let mut list = TabList::new();
        for _ in 0..n {
            list.add(true).unwrap();
        }
        list
    }

    #[test]
    fn a_new_list_has_no_tabs_and_no_active_one() {
        let mut list = TabList::new();
        assert!(list.is_empty());
        assert_eq!(list.active(), None);
        let state = list.snapshot();
        assert!(state.tabs.is_empty());
        assert_eq!(state.active, None);
    }

    #[test]
    fn every_snapshot_is_newer_than_the_one_before() {
        // The UI gets the same state from commands and from events, which can
        // arrive out of order: the revision says which one is newer.
        let mut list = list_of(1);
        let first = list.snapshot().revision;
        let second = list.snapshot().revision;
        list.close(1);
        let third = list.snapshot().revision;
        assert!(first > 0);
        assert!(second > first);
        assert!(third > second);
        list.clear();
        assert!(
            list.snapshot().revision > third,
            "a cleared list is newer too"
        );
    }

    #[test]
    fn the_first_tab_is_the_active_one_even_when_not_asked_to_be() {
        let mut list = TabList::new();
        assert_eq!(list.add(false), Ok(1));
        assert_eq!(list.active(), Some(1));
    }

    #[test]
    fn a_tab_added_in_the_foreground_becomes_active_and_one_in_the_background_does_not() {
        let mut list = list_of(1);
        assert_eq!(list.add(true), Ok(2));
        assert_eq!(list.active(), Some(2));
        assert_eq!(list.add(false), Ok(3));
        assert_eq!(list.active(), Some(2));
        assert_eq!(list.ids(), [1, 2, 3]);
    }

    #[test]
    fn a_new_tab_is_blank() {
        let list = list_of(1);
        assert_eq!(
            list.get(1),
            Some(&Tab {
                id: 1,
                url: None,
                title: None,
                blocked: None
            })
        );
    }

    #[test]
    fn the_fifth_tab_is_refused() {
        let mut list = list_of(MAX_TABS);
        assert_eq!(list.add(true), Err(Full));
        assert_eq!(list.len(), MAX_TABS);
        assert_eq!(list.active(), Some(MAX_TABS as u32));
    }

    #[test]
    fn closing_a_tab_makes_room_for_another() {
        let mut list = list_of(MAX_TABS);
        assert!(list.close(2));
        assert_eq!(list.add(true), Ok(5));
        assert_eq!(list.ids(), [1, 3, 4, 5]);
    }

    #[test]
    fn ids_are_never_reused() {
        let mut list = list_of(2);
        list.close(2);
        assert_eq!(list.add(true), Ok(3));
        list.close(3);
        list.close(1);
        // The only tab left was replaced: its successor has a new id too.
        assert_eq!(list.ids(), [4]);
        list.clear();
        assert_eq!(list.add(true), Ok(5));
    }

    #[test]
    fn activating_a_tab_picks_it_and_an_unknown_one_changes_nothing() {
        let mut list = list_of(3);
        assert!(list.activate(1));
        assert_eq!(list.active(), Some(1));
        assert!(!list.activate(9));
        assert_eq!(list.active(), Some(1));
    }

    #[test]
    fn closing_the_active_tab_activates_its_right_neighbour() {
        let mut list = list_of(3);
        list.activate(2);
        assert!(list.close(2));
        assert_eq!(list.ids(), [1, 3]);
        assert_eq!(list.active(), Some(3));
    }

    #[test]
    fn closing_the_last_active_tab_activates_its_left_neighbour() {
        let mut list = list_of(3);
        assert_eq!(list.active(), Some(3));
        assert!(list.close(3));
        assert_eq!(list.active(), Some(2));
    }

    #[test]
    fn closing_a_background_tab_keeps_the_active_one() {
        let mut list = list_of(3);
        list.activate(3);
        assert!(list.close(1));
        assert_eq!(list.active(), Some(3));
        assert_eq!(list.ids(), [2, 3]);
    }

    #[test]
    fn closing_the_only_tab_leaves_a_fresh_blank_one() {
        let mut list = list_of(1);
        list.get_mut(1).unwrap().url = Some("https://a.test/".into());
        assert!(list.close(1));
        assert_eq!(list.ids(), [2]);
        assert_eq!(list.active(), Some(2));
        assert_eq!(list.get(2).unwrap().url, None);
    }

    #[test]
    fn closing_an_unknown_tab_changes_nothing() {
        let mut list = list_of(2);
        let before = list.snapshot();
        assert!(!list.close(9));
        let after = list.snapshot();
        assert_eq!((after.tabs, after.active), (before.tabs, before.active));
    }

    #[test]
    fn clearing_forgets_every_tab() {
        let mut list = list_of(3);
        list.clear();
        assert!(list.is_empty());
        assert_eq!(list.active(), None);
        assert!(!list.close(1));
    }

    #[test]
    fn the_snapshot_reaches_the_ui_in_camel_case_with_nothing_extra() {
        let mut list = list_of(2);
        list.get_mut(1).unwrap().url = Some("https://a.test/".into());
        list.get_mut(1).unwrap().title = Some("A".into());
        list.get_mut(2).unwrap().blocked = Some("nope".into());
        let json = serde_json::to_value(list.snapshot()).unwrap();
        assert_eq!(json["active"], 2);
        let first = json["tabs"][0].as_object().unwrap();
        assert_eq!(first["id"], 1);
        assert_eq!(first["url"], "https://a.test/");
        assert_eq!(first["title"], "A");
        assert_eq!(first["blocked"], serde_json::Value::Null);
        assert_eq!(first.len(), 4, "an unexpected field would leak: {first:?}");
        assert_eq!(json["tabs"][1]["blocked"], "nope");
        assert!(json["revision"].as_u64().is_some_and(|n| n > 0));
        assert_eq!(json.as_object().unwrap().len(), 3);
    }
}
