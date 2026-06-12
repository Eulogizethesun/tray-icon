use std::cell::RefCell;

#[derive(Debug, Clone)]
pub struct PlatformIcon {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl PlatformIcon {
    pub fn from_rgba(rgba: Vec<u8>, width: u32, height: u32) -> Result<Self, crate::icon::BadIcon> {
        let expected = width as usize * height as usize * 4;
        if rgba.len() != expected {
            let pixel_count = rgba.len() / 4;
            return Err(crate::icon::BadIcon::DimensionsVsPixelCount {
                width,
                height,
                width_x_height: (width * height) as usize,
                pixel_count,
            });
        }
        Ok(Self {
            rgba,
            width,
            height,
        })
    }
}

pub fn icon_to_status_bar_icon(
    icon: &PlatformIcon,
    is_template: bool,
) -> crate::Result<openharmony_ability::statusbar::StatusBarIcon> {
    let rgba = &icon.rgba;
    let (width, height) = (icon.width, icon.height);

    let size = width.min(height);
    let scaled_rgba = if width != size || height != size {
        scale_rgba(rgba, width, height, size, size)
    } else {
        rgba.clone()
    };

    let (white, black) = if is_template {
        (
            RefCell::new(Some(to_monochrome(&scaled_rgba, 255))),
            RefCell::new(Some(to_monochrome(&scaled_rgba, 0))),
        )
    } else {
        (
            RefCell::new(Some(scaled_rgba.clone())),
            RefCell::new(Some(scaled_rgba)),
        )
    };

    Ok(openharmony_ability::statusbar::StatusBarIcon {
        white,
        black,
        size,
    })
}

/// Convert icon to a monochrome alpha mask, matching macOS NSImage template semantics.
///
/// On macOS, template images use the alpha channel as a mask and the system
/// applies a tint color. On OHOS, we simulate this by generating solid-color
/// silhouettes where the alpha channel defines the shape.
///
/// Semi-transparent pixels (anti-aliased edges) are thresholded to avoid
/// visible gray halos when the system renders against contrasting backgrounds.
fn to_monochrome(rgba: &[u8], color: u8) -> Vec<u8> {
    // Alpha threshold: pixels below this become fully transparent,
    // pixels at or above become fully opaque. This eliminates semi-transparent
    // edge pixels that cause gray halos on OHOS (where the system alpha-blends
    // white [255,255,255,N<255] against dark backgrounds).
    const ALPHA_THRESHOLD: u8 = 128;

    rgba.chunks(4)
        .flat_map(|pixel| {
            let a = pixel[3];
            if a < ALPHA_THRESHOLD {
                [0, 0, 0, 0]
            } else {
                [color, color, color, 255]
            }
        })
        .collect()
}



fn scale_rgba(rgba: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    if src_w == dst_w && src_h == dst_h {
        return rgba.to_vec();
    }

    let mut result = Vec::with_capacity(dst_w as usize * dst_h as usize * 4);

    for dst_y in 0..dst_h {
        for dst_x in 0..dst_w {
            let src_x = (dst_x as f32 * src_w as f32 / dst_w as f32) as u32;
            let src_y = (dst_y as f32 * src_h as f32 / dst_h as f32) as u32;
            let src_idx = (src_y * src_w + src_x) as usize * 4;

            result.push(rgba[src_idx]);
            result.push(rgba[src_idx + 1]);
            result.push(rgba[src_idx + 2]);
            result.push(rgba[src_idx + 3]);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scale_rgba_same_size() {
        let rgba = vec![10, 20, 30, 255, 40, 50, 60, 255];
        let scaled = scale_rgba(&rgba, 2, 1, 2, 1);
        assert_eq!(scaled, rgba);
    }

    #[test]
    fn test_scale_rgba_downsample() {
        let rgba = vec![255u8; 48 * 48 * 4];
        let scaled = scale_rgba(&rgba, 48, 48, 24, 24);
        assert_eq!(scaled.len(), 24 * 24 * 4);
        assert!(scaled.iter().all(|&b| b == 255));
    }

    #[test]
    fn test_scale_rgba_uses_nearest_neighbor() {
        let rgba = vec![
            100, 100, 100, 255, 200, 200, 200, 255, 150, 150, 150, 255, 250, 250, 250, 255,
        ];
        let scaled = scale_rgba(&rgba, 2, 2, 1, 1);
        assert_eq!(scaled.len(), 4);
        assert_eq!(scaled[0], 100);
        assert_eq!(scaled[1], 100);
        assert_eq!(scaled[2], 100);
        assert_eq!(scaled[3], 255);
    }

    #[test]
    fn test_to_monochrome_white_threshold() {
        let rgba = vec![
            255, 0, 0, 255,    // red, opaque → white, opaque
            0, 128, 255, 128,  // blue, alpha=128 (at threshold) → white, opaque
            0, 0, 0, 127,      // alpha=127 (below threshold) → transparent
            0, 0, 0, 0,        // transparent → transparent
        ];
        let white = to_monochrome(&rgba, 255);
        assert_eq!(white, vec![
            255, 255, 255, 255,
            255, 255, 255, 255,
            0, 0, 0, 0,
            0, 0, 0, 0,
        ]);
    }

    #[test]
    fn test_to_monochrome_black_threshold() {
        let rgba = vec![
            255, 255, 255, 200, // white pixel, alpha=200 → black, opaque
            50, 100, 150, 64,   // colored pixel, alpha=64 → transparent
        ];
        let black = to_monochrome(&rgba, 0);
        assert_eq!(black, vec![
            0, 0, 0, 255,
            0, 0, 0, 0,
        ]);
    }

    #[test]
    fn test_icon_to_status_bar_icon_template_different() {
        let icon = PlatformIcon {
            rgba: vec![255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 128, 128, 128, 64],
            width: 2,
            height: 2,
        };
        let result = icon_to_status_bar_icon(&icon, true).unwrap();
        let white = result.white.borrow().clone().unwrap();
        let black = result.black.borrow().clone().unwrap();
        // White version: all RGB=255, alpha thresholded to 255
        assert_eq!(white[0], 255); assert_eq!(white[1], 255); assert_eq!(white[2], 255); assert_eq!(white[3], 255);
        assert_eq!(white[4], 255); assert_eq!(white[5], 255); assert_eq!(white[6], 255); assert_eq!(white[7], 255);
        // Black version: all RGB=0, alpha thresholded to 255
        assert_eq!(black[0], 0); assert_eq!(black[1], 0); assert_eq!(black[2], 0); assert_eq!(black[3], 255);
        assert_eq!(black[4], 0); assert_eq!(black[5], 0); assert_eq!(black[6], 0); assert_eq!(black[7], 255);
        // They should be different
        assert_ne!(white, black);
    }

    #[test]
    fn test_icon_to_status_bar_icon_not_template_same() {
        let icon = PlatformIcon {
            rgba: vec![255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 128, 128, 128, 64],
            width: 2,
            height: 2,
        };
        let result = icon_to_status_bar_icon(&icon, false).unwrap();
        let white = result.white.borrow().clone().unwrap();
        let black = result.black.borrow().clone().unwrap();
        // Both should be the original image
        assert_eq!(white, black);
        assert_eq!(white, icon.rgba);
    }

    }
