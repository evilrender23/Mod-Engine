//! Shared, proportional zoom for the canvases (room, sprite, workspace).
//!
//! egui spreads one mouse-wheel notch over several frames (smooth scrolling), so
//! zoom must be *proportional* to each frame's delta, never a fixed step per frame.
//! An exponential mapping makes the per-frame factors multiply to the same total
//! no matter how the notch is split: `Π exp(k·dᵢ) = exp(k·Σdᵢ)`.

/// Exponent per point of scroll: one wheel notch (40 points on native, egui's
/// `line_scroll_speed`) zooms by ~10 % (`exp(K · 40) = 1.1`).
pub const K: f32 = 0.002_382_8; // ln(1.1) / 40

/// Multiplicative zoom factor for this frame.
///
/// `scroll_y` is the (smooth) vertical wheel delta in points, positive = zoom in;
/// `zoom_delta` is egui's per-frame pinch / Ctrl+wheel factor (1.0 = none).
pub fn factor(scroll_y: f32, zoom_delta: f32) -> f32 {
    (scroll_y * K).exp() * zoom_delta.clamp(0.5, 2.0)
}

/// Make egui's own Ctrl+wheel → `zoom_delta` conversion just as gentle (default is ~22 %/notch).
pub fn install(ctx: &eframe::egui::Context) {
    ctx.options_mut(|o| o.input_options.scroll_zoom_speed = K);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Points of scroll delta egui reports for one wheel notch on native.
    const NOTCH: f32 = 40.0;

    #[test]
    fn one_notch_is_about_ten_percent() {
        assert!((factor(NOTCH, 1.0) - 1.1).abs() < 1e-3);
        assert!((factor(-NOTCH, 1.0) - 1.0 / 1.1).abs() < 1e-3);
        assert_eq!(factor(0.0, 1.0), 1.0);
    }

    #[test]
    fn split_over_frames_gives_same_total() {
        let whole = factor(NOTCH, 1.0);
        let split: f32 = (0..10).map(|_| factor(NOTCH / 10.0, 1.0)).product();
        assert!((whole - split).abs() < 1e-4);
        // Uneven smoothing curve too.
        let parts = [12.0, 9.0, 7.0, 5.0, 3.5, 2.0, 1.0, 0.5];
        let total: f32 = parts.iter().sum();
        let uneven: f32 = parts.iter().map(|d| factor(*d, 1.0)).product();
        assert!((factor(total, 1.0) - uneven).abs() < 1e-4);
    }

    #[test]
    fn pinch_is_proportional_and_clamped() {
        assert!((factor(0.0, 1.05) - 1.05).abs() < 1e-6);
        assert_eq!(factor(0.0, 10.0), 2.0);
        assert_eq!(factor(0.0, 0.01), 0.5);
    }
}
