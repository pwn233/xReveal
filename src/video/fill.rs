//! Where the picture lands on the surface for each Fill mode (docs/spec/v1.md §2.5).
//!
//! Only Fit exists so far; the other Fill modes arrive with the full video shader.

/// A rectangle in surface pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Fit: the largest rectangle with the picture's true shape that fits the surface, centred.
/// The rest of the surface stays black.
pub fn fit(picture_width: f64, picture_height: f64, surface_width: f64, surface_height: f64) -> Rect {
    if picture_width <= 0.0 || picture_height <= 0.0 || surface_width <= 0.0 || surface_height <= 0.0 {
        return Rect { x: 0.0, y: 0.0, width: surface_width.max(0.0), height: surface_height.max(0.0) };
    }
    let scale = (surface_width / picture_width).min(surface_height / picture_height);
    let width = picture_width * scale;
    let height = picture_height * scale;
    Rect { x: (surface_width - width) / 2.0, y: (surface_height - height) / 2.0, width, height }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_shape_fills_the_surface() {
        assert_eq!(fit(1280.0, 720.0, 2560.0, 1440.0), Rect { x: 0.0, y: 0.0, width: 2560.0, height: 1440.0 });
    }

    #[test]
    fn wider_surface_gets_side_bars() {
        // 16:9 picture on a 21:9 ultrawide (3440x1440).
        let r = fit(1920.0, 1080.0, 3440.0, 1440.0);
        assert_eq!(r.height, 1440.0);
        assert_eq!(r.width, 2560.0);
        assert_eq!(r.x, 440.0);
        assert_eq!(r.y, 0.0);
    }

    #[test]
    fn taller_surface_gets_top_and_bottom_bars() {
        let r = fit(1920.0, 1080.0, 1600.0, 1200.0);
        assert_eq!(r.width, 1600.0);
        assert_eq!(r.height, 900.0);
        assert_eq!(r.y, 150.0);
    }

    #[test]
    fn empty_sizes_do_not_divide_by_zero() {
        assert_eq!(fit(0.0, 0.0, 800.0, 600.0), Rect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 });
    }
}
