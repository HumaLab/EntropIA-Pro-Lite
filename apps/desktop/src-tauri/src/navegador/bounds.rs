//! The rectangle the child webview occupies inside the app window.
//!
//! The frontend measures a placeholder element and sends its rect; this
//! validates it before it reaches the native layer, because a NaN, a negative
//! size or an absurd extent handed to the window system misbehaves in
//! platform-specific ways.

/// Largest coordinate or extent accepted, in logical pixels.
pub const MAX_EXTENT: f64 = 32_768.0;

/// A validated rectangle in logical pixels, relative to the window's client area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Validate a rect coming from the frontend.
pub fn sanitize(x: f64, y: f64, width: f64, height: f64) -> Result<Bounds, String> {
    if ![x, y, width, height].iter().all(|n| n.is_finite()) {
        return Err("The browser area has a non-numeric position or size".to_string());
    }
    if width < 1.0 || height < 1.0 {
        return Err("The browser area is smaller than one pixel".to_string());
    }
    if x.abs() > MAX_EXTENT || y.abs() > MAX_EXTENT || width > MAX_EXTENT || height > MAX_EXTENT {
        return Err("The browser area is out of range".to_string());
    }
    Ok(Bounds {
        x,
        y,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_rect_passes_unchanged() {
        assert_eq!(
            sanitize(10.5, 48.0, 800.0, 600.25),
            Ok(Bounds {
                x: 10.5,
                y: 48.0,
                width: 800.0,
                height: 600.25
            })
        );
    }

    #[test]
    fn a_rect_partly_off_screen_keeps_its_negative_origin() {
        // A scrolled or animating pane can start above or left of the window.
        assert!(sanitize(-20.0, -5.0, 400.0, 300.0).is_ok());
    }

    #[test]
    fn non_finite_numbers_are_rejected() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(sanitize(bad, 0.0, 10.0, 10.0).is_err());
            assert!(sanitize(0.0, bad, 10.0, 10.0).is_err());
            assert!(sanitize(0.0, 0.0, bad, 10.0).is_err());
            assert!(sanitize(0.0, 0.0, 10.0, bad).is_err());
        }
    }

    #[test]
    fn a_size_below_one_pixel_is_rejected() {
        assert!(sanitize(0.0, 0.0, 0.0, 10.0).is_err());
        assert!(sanitize(0.0, 0.0, 10.0, 0.5).is_err());
        assert!(sanitize(0.0, 0.0, -10.0, 10.0).is_err());
        assert!(sanitize(0.0, 0.0, 1.0, 1.0).is_ok());
    }

    #[test]
    fn extents_beyond_the_maximum_are_rejected() {
        assert!(sanitize(0.0, 0.0, MAX_EXTENT, MAX_EXTENT).is_ok());
        assert!(sanitize(0.0, 0.0, MAX_EXTENT + 1.0, 10.0).is_err());
        assert!(sanitize(0.0, 0.0, 10.0, MAX_EXTENT + 1.0).is_err());
        assert!(sanitize(MAX_EXTENT + 1.0, 0.0, 10.0, 10.0).is_err());
        assert!(sanitize(0.0, -MAX_EXTENT - 1.0, 10.0, 10.0).is_err());
    }
}
