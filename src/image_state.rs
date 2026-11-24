use image::RgbaImage;

pub const WHITE: image::Rgba<u8> = image::Rgba([255, 255, 255, 255]);
pub const DATA_CHANNEL_CUTOFF: u8 = 250;

#[derive(Clone, Debug)]
pub struct ImageState {
    pub original: RgbaImage,
    pub filtered: RgbaImage,
    pub edited: RgbaImage,
    pub version: u64,
}

impl ImageState {
    pub fn new(original: RgbaImage) -> Self {
        Self {
            filtered: original.clone(),
            edited: original.clone(),
            original,
            version: 0,
        }
    }

    pub fn apply_filter(&mut self, filtered: RgbaImage) {
        self.filtered = filtered.clone();
        self.edited = filtered;
        self.bump();
    }

    pub fn reset_filter(&mut self) {
        self.filtered = self.original.clone();
        self.edited = self.original.clone();
        self.bump();
    }

    pub fn bump(&mut self) {
        self.version += 1;
    }

    pub fn dimensions(&self) -> (u32, u32) {
        self.original.dimensions()
    }
}

pub fn is_data_px(p: &image::Rgba<u8>) -> bool {
    p.0[0] < DATA_CHANNEL_CUTOFF || p.0[1] < DATA_CHANNEL_CUTOFF || p.0[2] < DATA_CHANNEL_CUTOFF
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, WHITE)
    }

    #[test]
    fn new_state_shares_content() {
        let base = img(4, 4);
        let st = ImageState::new(base.clone());
        assert_eq!(st.original.dimensions(), (4, 4));
        assert_eq!(st.filtered.get_pixel(2, 2), st.original.get_pixel(2, 2));
        assert_eq!(st.edited.get_pixel(2, 2), st.original.get_pixel(2, 2));
        assert_eq!(st.version, 0);
    }

    #[test]
    fn apply_filter_resets_edited() {
        let base = img(4, 4);
        let mut st = ImageState::new(base.clone());
        st.edited.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
        st.bump();
        let filtered = {
            let mut f = base.clone();
            f.put_pixel(1, 1, image::Rgba([10, 10, 10, 255]));
            f
        };
        let v_before = st.version;
        st.apply_filter(filtered.clone());
        assert_eq!(st.filtered.get_pixel(1, 1), &image::Rgba([10, 10, 10, 255]));
        assert_eq!(st.edited.get_pixel(0, 0), &WHITE);
        assert!(st.version > v_before);
    }

    #[test]
    fn reset_filter_restores_original() {
        let base = img(4, 4);
        let mut st = ImageState::new(base.clone());
        st.original.put_pixel(3, 3, image::Rgba([1, 2, 3, 255]));
        st.reset_filter();
        assert_eq!(st.filtered.get_pixel(3, 3), &image::Rgba([1, 2, 3, 255]));
        assert_eq!(st.edited.get_pixel(3, 3), &image::Rgba([1, 2, 3, 255]));
    }

    #[test]
    fn is_data_px_thresholds() {
        assert!(is_data_px(&image::Rgba([0, 0, 0, 255])));
        assert!(is_data_px(&image::Rgba([200, 30, 40, 255])));
        assert!(is_data_px(&image::Rgba([249, 255, 255, 255])));
        assert!(!is_data_px(&WHITE));
        assert!(!is_data_px(&image::Rgba([250, 250, 250, 255])));
    }
}
