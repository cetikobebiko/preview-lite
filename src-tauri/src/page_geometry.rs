//! Sayfanın görüntü geometrisi: PDF kullanıcı uzayı ile ekranda gösterilen (CropBox'a kırpılmış,
//! `/Rotate` uygulanmış) sayfa arasındaki dönüşüm.
//!
//! PDFium bir sayfayı `FPDF_GetPageBoundingBox` kutusuyla (CropBox ∩ MediaBox) render eder ve
//! `/Rotate`'i saat yönünde uygular. Frontend'in metin katmanı, arama vurguları ve OCR metin
//! katmanı aynı görüntü uzayını kullanır: sol üst köşe başlangıç, aşağı doğru artan y.
//! Kapalı formül `FPDF_DeviceToPage` ile birebir aynıdır (bkz. `searchable_pdf` testleri).

use pdfium_render::prelude::*;

/// Kapalı formülün PDFium'un sayfa boyutuyla izinli farkı (pt).
const SIZE_TOLERANCE_PT: f64 = 0.01;

/// Sayfanın görüntü çerçevesi: PDFium'un render ettiği kutu ve `/Rotate` (çeyrek tur, saat yönünde).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PageFrame {
    pub(crate) rotation: u8,
    /// `[x0, y0, x1, y1]`, kullanıcı uzayında.
    pub(crate) bbox: [f64; 4],
}

impl PageFrame {
    /// PDFium'un bu sayfa için kullandığı çerçeve. Kutu okunamazsa ya da PDFium'un döndürülmüş
    /// sayfa boyutuyla uyuşmazsa `None`.
    pub(crate) fn of_page(bindings: &dyn PdfiumLibraryBindings, page: FPDF_PAGE) -> Option<Self> {
        let mut rect = FS_RECTF {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        };
        if !bindings.is_true(bindings.FPDF_GetPageBoundingBox(page, &mut rect)) {
            return None;
        }
        let (left, right) = (f64::from(rect.left), f64::from(rect.right));
        let (bottom, top) = (f64::from(rect.bottom), f64::from(rect.top));
        let frame = Self {
            rotation: bindings.FPDFPage_GetRotation(page).rem_euclid(4) as u8,
            bbox: [
                left.min(right),
                bottom.min(top),
                left.max(right),
                bottom.max(top),
            ],
        };
        let (width, height) = frame.display_size();
        let page_width = f64::from(bindings.FPDF_GetPageWidthF(page));
        let page_height = f64::from(bindings.FPDF_GetPageHeightF(page));
        let consistent = width > 0.0
            && height > 0.0
            && frame.bbox.iter().all(|value| value.is_finite())
            && (page_width - width).abs() <= SIZE_TOLERANCE_PT
            && (page_height - height).abs() <= SIZE_TOLERANCE_PT;
        consistent.then_some(frame)
    }

    /// [`Self::of_page`] for a pdfium-render page.
    pub(crate) fn of(bindings: &dyn PdfiumLibraryBindings, page: &PdfPage<'_>) -> Option<Self> {
        Self::of_page(bindings, bindings.get_handle_from_page(page))
    }

    /// Döndürülmüş sayfanın ekrandaki genişliği ve yüksekliği (pt).
    pub(crate) fn display_size(&self) -> (f64, f64) {
        let width = self.bbox[2] - self.bbox[0];
        let height = self.bbox[3] - self.bbox[1];
        if self.rotation % 2 == 1 {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// Ekran noktası (döndürülmüş sayfanın sol üst köşesinden, aşağı doğru pt) → kullanıcı uzayı.
    pub(crate) fn to_user(self, u: f64, v: f64) -> (f64, f64) {
        let [x0, y0, x1, y1] = self.bbox;
        match self.rotation {
            0 => (x0 + u, y1 - v),
            1 => (x0 + v, y0 + u),
            2 => (x1 - u, y0 + v),
            _ => (x1 - v, y1 - u),
        }
    }

    /// Kullanıcı uzayı noktası → ekran noktası; [`Self::to_user`]'ın tersi.
    pub(crate) fn to_display(self, x: f64, y: f64) -> (f64, f64) {
        let [x0, y0, x1, y1] = self.bbox;
        match self.rotation {
            0 => (x - x0, y1 - y),
            1 => (y - y0, x - x0),
            2 => (x1 - x, y - y0),
            _ => (y1 - y, x1 - x),
        }
    }

    /// Kullanıcı uzayında eksene hizalı kutu `[min_x, min_y, max_x, max_y]` → görüntü uzayında
    /// sayfaya göre normalize `[x, y, width, height]` (sol üst köşe başlangıç). Değerler
    /// sonlu değilse `None`.
    pub(crate) fn normalized_rect(
        self,
        [min_x, min_y, max_x, max_y]: [f64; 4],
    ) -> Option<[f64; 4]> {
        let (width, height) = self.display_size();
        let (u0, v0) = self.to_display(min_x, min_y);
        let (u1, v1) = self.to_display(max_x, max_y);
        let rect = [
            u0.min(u1) / width,
            v0.min(v1) / height,
            (u0 - u1).abs() / width,
            (v0 - v1).abs() / height,
        ];
        rect.iter().all(|value| value.is_finite()).then_some(rect)
    }
}

/// Bir karakterin yazı yönü, çeyrek tura yuvarlanmış: 0 → +x, 1 → +y, 2 → -x, 3 → -y
/// (kullanıcı uzayında). `FPDFText_GetCharAngle` saat yönünün tersine radyan verir.
pub(crate) fn quarter_turn(angle_radians: f32) -> u8 {
    if !angle_radians.is_finite() {
        return 0;
    }
    let quarters = (f64::from(angle_radians) / std::f64::consts::FRAC_PI_2).round() as i64;
    quarters.rem_euclid(4) as u8
}

/// Kullanıcı uzayı kutusunu yazı yönüne hizalı eksenlere çevirir: sonuçta yazı +x yönünde,
/// satırın "yukarısı" +y yönündedir. Sözcük ve satır ayırma bu eksenlerde yapılır.
pub(crate) fn text_axis_box([min_x, min_y, max_x, max_y]: [f32; 4], turn: u8) -> [f32; 4] {
    match turn % 4 {
        0 => [min_x, min_y, max_x, max_y],
        1 => [min_y, -max_x, max_y, -min_x],
        2 => [-max_x, -max_y, -min_x, -min_y],
        _ => [-max_y, min_x, -min_y, max_x],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(rotation: u8) -> PageFrame {
        PageFrame {
            rotation,
            bbox: [10.0, 20.0, 110.0, 220.0],
        }
    }

    #[test]
    fn to_display_inverts_to_user_for_every_rotation_and_crop_offset() {
        for rotation in 0..4 {
            let frame = frame(rotation);
            for (u, v) in [(0.0, 0.0), (37.5, 12.25), (99.0, 3.0)] {
                let (x, y) = frame.to_user(u, v);
                assert_eq!(frame.to_display(x, y), (u, v), "rotation {rotation}");
            }
        }
    }

    #[test]
    fn normalized_rect_is_in_displayed_page_space() {
        // A box near the user-space bottom-left corner of the crop box.
        let user = [10.0, 20.0, 30.0, 25.0];
        // r0: bottom-left of the display; r1 (90° cw): top-left; r2: top-right; r3: bottom-right.
        let expected = [
            [0.0, 195.0 / 200.0, 0.2, 5.0 / 200.0],
            [0.0, 0.0, 5.0 / 200.0, 0.2],
            [0.8, 0.0, 0.2, 5.0 / 200.0],
            [195.0 / 200.0, 0.8, 5.0 / 200.0, 0.2],
        ];
        for rotation in 0..4u8 {
            let rect = frame(rotation).normalized_rect(user).unwrap();
            for (got, want) in rect.iter().zip(expected[rotation as usize]) {
                assert!((got - want).abs() < 1e-12, "rotation {rotation}: {rect:?}");
            }
        }
    }

    #[test]
    fn quarter_turn_rounds_char_angles() {
        use std::f32::consts::{FRAC_PI_2, PI};
        assert_eq!(quarter_turn(0.0), 0);
        assert_eq!(quarter_turn(0.2), 0);
        assert_eq!(quarter_turn(FRAC_PI_2), 1);
        assert_eq!(quarter_turn(PI), 2);
        assert_eq!(quarter_turn(3.0 * FRAC_PI_2), 3);
        assert_eq!(quarter_turn(2.0 * PI - 0.1), 0);
        assert_eq!(quarter_turn(f32::NAN), 0);
    }

    #[test]
    fn text_axis_box_puts_the_reading_direction_on_x() {
        // Two characters advancing along user +y (turn 1): in text axes they advance along +x.
        let first = text_axis_box([100.0, 10.0, 112.0, 18.0], 1);
        let second = text_axis_box([100.0, 19.0, 112.0, 27.0], 1);
        assert!(second[0] > first[2], "next character is further along x");
        assert_eq!(first[1], second[1], "same line in text axes");
        // Advancing along -x (turn 2) and -y (turn 3).
        let a = text_axis_box([50.0, 0.0, 58.0, 12.0], 2);
        let b = text_axis_box([40.0, 0.0, 48.0, 12.0], 2);
        assert!(b[0] > a[2]);
        let c = text_axis_box([0.0, 50.0, 12.0, 58.0], 3);
        let d = text_axis_box([0.0, 40.0, 12.0, 48.0], 3);
        assert!(d[0] > c[2]);
        for turn in 0..4 {
            let [x0, y0, x1, y1] = text_axis_box([1.0, 2.0, 4.0, 8.0], turn);
            assert!(x0 < x1 && y0 < y1, "turn {turn}");
        }
    }
}
