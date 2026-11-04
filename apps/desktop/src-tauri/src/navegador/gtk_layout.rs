//! Linux only: puts the tabs' webviews over the app instead of beside it.
//!
//! On WebKitGTK, Tauri packs every webview of a window into the window's
//! vertical `GtkBox` (tauri-runtime-wry `create_webview`: `build_gtk(vbox)`),
//! so a child webview takes half the window below the app's webview, and
//! `Webview::set_bounds` does nothing for a webview inside a box (wry moves only
//! one inside a `GtkFixed`). Windows and macOS place a child at its rect.
//!
//! So, at startup and while the window is still hidden, [`wrap_app_webview`]
//! puts the app's webview in a `GtkOverlay` that takes its place in the box;
//! [`place`] moves a tab's webview from the box onto that overlay and gives it
//! the rect the view measured, as margins from the top-left corner and a size
//! request. Showing, hiding and closing stay Tauri's: wry shows, hides and
//! destroys the widget wherever it is.
//!
//! Everything here touches GTK widgets, so it runs on the main thread only
//! (inside `with_webview`), which is also why the overlay is a thread local.

use std::cell::RefCell;

use gtk::prelude::*;

use super::bounds::Bounds;

thread_local! {
    static OVERLAY: RefCell<Option<gtk::Overlay>> = const { RefCell::new(None) };
}

/// Wrap the app's webview in the overlay the tabs go on. Idempotent.
pub fn wrap_app_webview(webview: &gtk::Widget) -> Result<(), String> {
    if OVERLAY.with(|overlay| overlay.borrow().is_some()) {
        return Ok(());
    }
    let container = webview
        .parent()
        .ok_or_else(|| "The app webview has no container".to_string())?
        .downcast::<gtk::Box>()
        .map_err(|_| "The app webview is not in a box".to_string())?;
    let position = container
        .children()
        .iter()
        .position(|child| child == webview)
        .ok_or_else(|| "The app webview is not a child of its container".to_string())?;
    let focused = webview.has_focus();

    let overlay = gtk::Overlay::new();
    // `webview` holds its own reference, so the widget outlives the removal.
    container.remove(webview);
    overlay.add(webview);
    container.pack_start(&overlay, true, true, 0);
    container.reorder_child(&overlay, position as i32);
    overlay.show();
    if focused {
        webview.grab_focus();
    }
    OVERLAY.with(|slot| *slot.borrow_mut() = Some(overlay));
    Ok(())
}

/// Put a tab's webview on the overlay, if it is not there yet, at `bounds`.
pub fn place(webview: &gtk::Widget, bounds: Bounds) -> Result<(), String> {
    let overlay = OVERLAY
        .with(|overlay| overlay.borrow().clone())
        .ok_or_else(|| "The app webview was not prepared for the browser".to_string())?;
    let on_overlay = webview
        .parent()
        .is_some_and(|parent| &parent == overlay.upcast_ref::<gtk::Widget>());
    if !on_overlay {
        if let Some(parent) = webview.parent() {
            let parent = parent
                .downcast::<gtk::Container>()
                .map_err(|_| "The tab's webview is not in a container".to_string())?;
            parent.remove(webview);
        }
        webview.set_halign(gtk::Align::Start);
        webview.set_valign(gtk::Align::Start);
        overlay.add_overlay(webview);
    }
    let (x, y, width, height) = pixels(bounds);
    webview.set_margin_start(x);
    webview.set_margin_top(y);
    webview.set_size_request(width, height);
    Ok(())
}

/// The rect in whole logical pixels (GTK's unit). A margin cannot be negative,
/// so a rect that starts left of or above the window starts at its edge.
fn pixels(bounds: Bounds) -> (i32, i32, i32, i32) {
    (
        bounds.x.round().max(0.0) as i32,
        bounds.y.round().max(0.0) as i32,
        bounds.width.round().max(1.0) as i32,
        bounds.height.round().max(1.0) as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Bounds {
        Bounds {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn pixels_rounds_the_rect_to_whole_pixels() {
        assert_eq!(
            pixels(rect(36.4, 271.6, 1839.5, 233.2)),
            (36, 272, 1840, 233)
        );
    }

    #[test]
    fn pixels_starts_a_rect_off_the_top_left_at_the_window_edge() {
        assert_eq!(pixels(rect(-12.0, -0.6, 100.0, 50.0)), (0, 0, 100, 50));
    }

    #[test]
    fn pixels_keeps_a_sub_pixel_extent_one_pixel_wide() {
        assert_eq!(pixels(rect(0.0, 0.0, 1.2, 1.4)), (0, 0, 1, 1));
    }
}
