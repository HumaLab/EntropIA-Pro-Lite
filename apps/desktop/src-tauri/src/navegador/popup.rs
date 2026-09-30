//! Pop-up windows the page asks for (`window.open`, `target=_blank`), the pure
//! part: how they are named and how many may exist.
//!
//! Some flows cannot work in the same webview. Google Sign-In with
//! `ux_mode=popup` opens `accounts.google.com` in a window and reports back to
//! the page through `window.opener` and `postMessage`; load that address in the
//! page's own webview and the opener is gone, so the popup stays blank. So an
//! allowed popup is a real, separate window whose webview is the one the engine
//! asked for (see `viewer::new_window`).
//!
//! Those windows carry a label that starts with [`LABEL_PREFIX`] and, like the
//! browser's own webview (`navegador-web`), no capability at all: the ACL
//! refuses every command and plugin call from them (`tests/app_acl.rs`).

/// Every popup label starts with this. The prefix is what tells a popup from
/// the main window, the browser webview and any window the app might add.
pub const LABEL_PREFIX: &str = "navegador-popup-";

/// Popups that may be open at once. A page asking for more is refused: a
/// stream of windows is what a hostile page would open.
pub const MAX_OPEN: usize = 3;

/// The label of the `n`th popup this session (numbers are never reused, so two
/// windows never share a label even when one is still closing).
pub fn label(n: u32) -> String {
    format!("{LABEL_PREFIX}{n}")
}

/// Whether `label` is one this module could have made: the prefix and a plain
/// number, nothing else (`navegador-popup-1`, not `navegador-popup-x` or
/// `navegador-popup-1-evil`).
pub fn is_popup_label(label: &str) -> bool {
    label
        .strip_prefix(LABEL_PREFIX)
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

/// How many of `labels` are popups.
pub fn count_open<'a>(labels: impl IntoIterator<Item = &'a str>) -> usize {
    labels.into_iter().filter(|l| is_popup_label(l)).count()
}

/// Whether one more popup may open when `open` already exist.
pub fn has_room(open: usize) -> bool {
    open < MAX_OPEN
}

/// Shown when a page asks for a popup and the limit is reached.
pub const TOO_MANY_MESSAGE: &str = "The page tried to open too many windows";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_carry_the_prefix_and_a_number() {
        assert_eq!(label(1), "navegador-popup-1");
        assert_eq!(label(42), "navegador-popup-42");
        assert!(label(7).starts_with(LABEL_PREFIX));
    }

    #[test]
    fn labels_are_unique_per_number() {
        assert_ne!(label(1), label(2));
    }

    #[test]
    fn a_label_made_here_is_recognised() {
        for n in [0, 1, 9, 10, 4_000_000_000] {
            assert!(is_popup_label(&label(n)), "{n}");
        }
    }

    #[test]
    fn only_that_shape_counts_as_a_popup() {
        for other in [
            "main",
            "navegador-web",
            "navegador-popup-",
            "navegador-popup-x",
            "navegador-popup-1-evil",
            "navegador-popup-1 ",
            "navegador-popup--1",
            "Navegador-popup-1",
            "xnavegador-popup-1",
            "",
        ] {
            assert!(!is_popup_label(other), "{other:?}");
        }
    }

    #[test]
    fn open_popups_are_counted_among_all_windows() {
        let labels = [
            "main",
            "navegador-web",
            "navegador-popup-1",
            "navegador-popup-3",
            "splashscreen",
        ];
        assert_eq!(count_open(labels), 2);
        assert_eq!(count_open([]), 0);
    }

    #[test]
    fn a_page_gets_a_few_popups_and_no_more() {
        assert_eq!(MAX_OPEN, 3);
        assert!(has_room(0));
        assert!(has_room(MAX_OPEN - 1));
        assert!(!has_room(MAX_OPEN));
        assert!(!has_room(MAX_OPEN + 1));
    }
}
