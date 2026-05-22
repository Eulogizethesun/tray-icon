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
) -> crate::Result<openharmony_ability::statusbar::StatusBarIcon> {
    let rgba = &icon.rgba;
    let (width, height) = (icon.width, icon.height);

    let size = width.min(height);
    let scaled_rgba = if width != size || height != size {
        scale_rgba(rgba, width, height, size, size)
    } else {
        rgba.clone()
    };

    Ok(openharmony_ability::statusbar::StatusBarIcon {
        white: RefCell::new(Some(scaled_rgba.clone())),
        black: RefCell::new(Some(scaled_rgba)),
        size,
    })
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

    }
