//! Birden çok görselden PDF oluşturma: her görsel bir sayfa olur.
//!
//! Yeni bir PDF yazılır; kullanıcının PDF'i açılmaz. lopdf yalnız yeni belgeyi serileştirir
//! (AGENTS.md). JPEG dosyaları yeniden sıkıştırılmadan `DCTDecode` olarak gömülür (gri ya da
//! RGB, EXIF yönü düz olanlar); diğerleri çözülüp Flate ile gömülür, saydamlık `SMask` olur.
//! Çözme ve sıkıştırma ana iş parçacığında değil `spawn_blocking` içinde çalışır.

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, ObjectId, Stream};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::path::Path;

/// Bir seferde en fazla görsel.
pub const MAX_IMAGES: usize = 500;
/// Belleğe alınan sıkıştırılmış görsel verisinin toplam sınırı.
const MAX_TOTAL_IMAGE_BYTES: usize = 1024 * 1024 * 1024;
/// Görsel boyutunda sayfa: 96 dpi (1 piksel = 0,75 pt).
const POINTS_PER_PIXEL: f32 = 0.75;
/// PDF sayfa kenarı sınırı (200 inç).
const MAX_PAGE_POINTS: f32 = 14_400.0;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PageSizing {
    /// Sayfa görselin boyutundadır (96 dpi).
    Image,
    /// A4; yönü görsele göre seçilir, görsel kenar boşluğu içinde ortalanıp sığdırılır.
    A4,
    /// US Letter; A4 ile aynı yerleşim.
    Letter,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImagesToPdfOptions {
    pub page_size: PageSizing,
    /// Kâğıt boyutlarında kenar boşluğu (pt); görsel boyutunda yok sayılır.
    pub margin: f32,
}

/// Sayfaya gömülecek görsel: çözülmüş ya da olduğu gibi aktarılan JPEG.
struct EmbeddedImage {
    width: u32,
    height: u32,
    stream: Stream,
    soft_mask: Option<Stream>,
}

pub(crate) fn validate_request(
    paths: &[String],
    options: &ImagesToPdfOptions,
) -> Result<(), String> {
    if paths.is_empty() {
        return Err("En az bir görsel seçin".into());
    }
    if paths.len() > MAX_IMAGES {
        return Err(format!(
            "Bir seferde en fazla {MAX_IMAGES} görsel dönüştürülebilir"
        ));
    }
    if !options.margin.is_finite() || !(0.0..=144.0).contains(&options.margin) {
        return Err("Kenar boşluğu 0 ile 144 pt arasında olmalı".into());
    }
    Ok(())
}

/// `paths`'teki görsellerden bir PDF'in baytlarını üretir.
pub(crate) fn build_pdf(paths: &[String], options: &ImagesToPdfOptions) -> Result<Vec<u8>, String> {
    validate_request(paths, options)?;
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let mut kids = Vec::with_capacity(paths.len());
    let mut total_bytes = 0usize;
    for path in paths {
        let image = load_image(Path::new(path)).map_err(|error| {
            let name = Path::new(path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            format!("{name}: {error}")
        })?;
        total_bytes += image.stream.content.len()
            + image
                .soft_mask
                .as_ref()
                .map_or(0, |mask| mask.content.len());
        if total_bytes > MAX_TOTAL_IMAGE_BYTES {
            return Err("Görsellerin toplam boyutu 1 GiB sınırını aşıyor".into());
        }
        kids.push(add_page(&mut document, pages_id, image, options)?.into());
    }
    let count = kids.len() as i64;
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );
    let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    document
        .save_to(&mut bytes)
        .map_err(|error| format!("PDF yazılamadı: {error}"))?;
    Ok(bytes)
}

fn load_image(path: &Path) -> Result<EmbeddedImage, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("okunamadı: {error}"))?;
    if bytes.len() as u64 > crate::MAX_IMAGE_BYTES {
        return Err(crate::image_too_large_error());
    }
    let mut decoder = bounded_reader(&bytes)?
        .into_decoder()
        .map_err(|error| decode_error(&error))?;
    let (width, height) = decoder.dimensions();
    let rgba_bytes = u64::from(width) * u64::from(height) * 4;
    if decoder.total_bytes() > crate::MAX_IMAGE_BYTES || rgba_bytes > crate::MAX_IMAGE_BYTES {
        return Err(crate::image_too_large_error());
    }
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    if image::guess_format(&bytes).ok() == Some(ImageFormat::Jpeg)
        && orientation == image::metadata::Orientation::NoTransforms
    {
        if let Some(color_space) = jpeg_color_space(&bytes) {
            drop(decoder);
            return Ok(EmbeddedImage {
                width,
                height,
                stream: Stream::new(
                    image_dictionary(width, height, color_space, "DCTDecode"),
                    bytes,
                ),
                soft_mask: None,
            });
        }
    }
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(|error| decode_error(&error))?;
    decoded.apply_orientation(orientation);
    Ok(flate_image(&decoded))
}

fn bounded_reader(bytes: &[u8]) -> Result<ImageReader<Cursor<&[u8]>>, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("biçimi okunamadı: {error}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(crate::MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(crate::MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(crate::MAX_IMAGE_BYTES);
    reader.limits(limits);
    Ok(reader)
}

fn decode_error(error: &image::ImageError) -> String {
    match error {
        image::ImageError::Limits(_) => crate::image_too_large_error(),
        error => format!("çözülemedi: {error}"),
    }
}

/// JPEG'in `/ColorSpace`'i, olduğu gibi gömülebiliyorsa: 1 bileşen gri, 3 bileşen RGB.
/// CMYK (4 bileşen) ve tanınmayan çerçeveler çözülüp yeniden sıkıştırılır.
fn jpeg_color_space(bytes: &[u8]) -> Option<&'static str> {
    let mut index = 2;
    while index + 4 <= bytes.len() {
        if bytes[index] != 0xFF {
            return None;
        }
        let marker = bytes[index + 1];
        // Dolgu baytları.
        if marker == 0xFF {
            index += 1;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([bytes[index + 2], bytes[index + 3]]));
        // SOF0 (baseline), SOF1 (extended), SOF2 (progressive): bileşen sayısı 8. bayttadır.
        if matches!(marker, 0xC0..=0xC2) {
            return match bytes.get(index + 9)? {
                1 => Some("DeviceGray"),
                3 => Some("DeviceRGB"),
                _ => None,
            };
        }
        // Diğer SOF'lar (aritmetik, kayıpsız) PDF okuyucularında desteklenmez.
        if matches!(marker, 0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) || marker == 0xDA {
            return None;
        }
        index += 2 + length;
    }
    None
}

fn image_dictionary(width: u32, height: u32, color_space: &str, filter: &str) -> lopdf::Dictionary {
    dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => i64::from(width),
        "Height" => i64::from(height),
        "ColorSpace" => color_space,
        "BitsPerComponent" => 8,
        "Filter" => filter,
    }
}

fn flate_image(image: &DynamicImage) -> EmbeddedImage {
    let (width, height) = (image.width(), image.height());
    let gray = matches!(
        image.color(),
        image::ColorType::L8
            | image::ColorType::La8
            | image::ColorType::L16
            | image::ColorType::La16
    );
    let (color_space, pixels) = if gray {
        ("DeviceGray", image.to_luma8().into_raw())
    } else {
        ("DeviceRGB", image.to_rgb8().into_raw())
    };
    let soft_mask = image.color().has_alpha().then(|| {
        let alpha: Vec<u8> = image.to_rgba8().pixels().map(|pixel| pixel.0[3]).collect();
        Stream::new(
            image_dictionary(width, height, "DeviceGray", "FlateDecode"),
            miniz_oxide::deflate::compress_to_vec_zlib(&alpha, 6),
        )
    });
    EmbeddedImage {
        width,
        height,
        stream: Stream::new(
            image_dictionary(width, height, color_space, "FlateDecode"),
            miniz_oxide::deflate::compress_to_vec_zlib(&pixels, 6),
        ),
        soft_mask,
    }
}

/// Sayfa boyutu ve görselin sayfadaki dikdörtgeni (x, y, genişlik, yükseklik) pt olarak.
fn layout(width: u32, height: u32, options: &ImagesToPdfOptions) -> ((f32, f32), [f32; 4]) {
    let natural = (
        width as f32 * POINTS_PER_PIXEL,
        height as f32 * POINTS_PER_PIXEL,
    );
    let paper = match options.page_size {
        PageSizing::Image => {
            // Çok büyük görseller sayfa sınırına sığdırılır.
            let scale = (MAX_PAGE_POINTS / natural.0.max(natural.1)).min(1.0);
            let page = (natural.0 * scale, natural.1 * scale);
            return (page, [0.0, 0.0, page.0, page.1]);
        }
        PageSizing::A4 => (595.276, 841.89),
        PageSizing::Letter => (612.0, 792.0),
    };
    let page = if natural.0 > natural.1 {
        (paper.1, paper.0)
    } else {
        paper
    };
    let room = (
        (page.0 - 2.0 * options.margin).max(1.0),
        (page.1 - 2.0 * options.margin).max(1.0),
    );
    let scale = (room.0 / natural.0).min(room.1 / natural.1);
    let size = (natural.0 * scale, natural.1 * scale);
    (
        page,
        [
            (page.0 - size.0) / 2.0,
            (page.1 - size.1) / 2.0,
            size.0,
            size.1,
        ],
    )
}

fn add_page(
    document: &mut Document,
    pages_id: ObjectId,
    image: EmbeddedImage,
    options: &ImagesToPdfOptions,
) -> Result<ObjectId, String> {
    let ((page_width, page_height), [x, y, width, height]) =
        layout(image.width, image.height, options);
    let mut stream = image.stream;
    if let Some(mask) = image.soft_mask {
        let mask_id = document.add_object(mask);
        stream.dict.set("SMask", mask_id);
    }
    let image_id = document.add_object(stream);
    let content = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![
                    width.into(),
                    0.into(),
                    0.into(),
                    height.into(),
                    x.into(),
                    y.into(),
                ],
            ),
            Operation::new("Do", vec![Object::Name(b"Im1".to_vec())]),
            Operation::new("Q", vec![]),
        ],
    };
    let content = content
        .encode()
        .map_err(|error| format!("Sayfa içeriği kodlanamadı: {error}"))?;
    let content_id = document.add_object(Stream::new(dictionary! {}, content));
    Ok(document.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), page_width.into(), page_height.into()],
        "Contents" => content_id,
        "Resources" => dictionary! { "XObject" => dictionary! { "Im1" => image_id } },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_image(
        directory: &Path,
        name: &str,
        image: &DynamicImage,
        format: ImageFormat,
    ) -> String {
        let path = directory.join(name);
        image.save_with_format(&path, format).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn options(page_size: PageSizing) -> ImagesToPdfOptions {
        ImagesToPdfOptions {
            page_size,
            margin: 36.0,
        }
    }

    #[test]
    fn jpeg_is_embedded_as_is_and_png_alpha_becomes_a_soft_mask() {
        let directory = tempfile::tempdir().unwrap();
        let jpeg = write_image(
            directory.path(),
            "photo.jpg",
            &DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                40,
                20,
                image::Rgb([200, 10, 10]),
            )),
            ImageFormat::Jpeg,
        );
        let png = write_image(
            directory.path(),
            "logo.png",
            &DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                10,
                30,
                image::Rgba([0, 0, 255, 128]),
            )),
            ImageFormat::Png,
        );
        let bytes = build_pdf(&[jpeg.clone(), png], &options(PageSizing::Image)).unwrap();
        let document = Document::load_mem(&bytes).unwrap();
        assert_eq!(document.get_pages().len(), 2);
        let jpeg_bytes = std::fs::read(&jpeg).unwrap();
        let streams: Vec<&Stream> = document
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .collect();
        assert!(streams.iter().any(|stream| stream.content == jpeg_bytes
            && stream.dict.get(b"Filter").unwrap().as_name().unwrap() == b"DCTDecode"));
        assert!(streams.iter().any(|stream| stream.dict.has(b"SMask")));

        crate::pdf_engine::with_pdfium(|pdfium| -> Result<(), String> {
            let pdf = pdfium
                .load_pdf_from_byte_slice(&bytes, None)
                .map_err(|e| e.to_string())?;
            let size = pdf.pages().page_size(0).map_err(|e| e.to_string())?;
            assert!((size.width().value - 30.0).abs() < 0.01);
            assert!((size.height().value - 15.0).abs() < 0.01);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn paper_sizes_turn_with_the_image_and_keep_the_margin() {
        let ((width, height), [x, y, w, h]) = layout(4000, 3000, &options(PageSizing::A4));
        assert!(width > height, "landscape image on landscape A4");
        assert!((x - 36.0).abs() < 0.01 || (y - 36.0).abs() < 0.01);
        assert!((w / h - 4.0 / 3.0).abs() < 0.001);
        assert!(x >= 36.0 - 0.01 && y >= 36.0 - 0.01);
        let ((width, height), _) = layout(100, 200, &options(PageSizing::Letter));
        assert_eq!((width, height), (612.0, 792.0));
    }

    #[test]
    fn cmyk_jpegs_are_reencoded_but_gray_ones_pass_through() {
        // A minimal SOF0 header with four components.
        let mut cmyk = vec![
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x10, 0x00, 0x10, 0x04,
        ];
        cmyk.extend([0; 12]);
        assert_eq!(jpeg_color_space(&cmyk), None);
        let gray = [
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x10, 0x00, 0x10, 0x01, 0, 0, 0,
        ];
        assert_eq!(jpeg_color_space(&gray), Some("DeviceGray"));
    }

    #[test]
    fn requests_are_bounded_and_bad_files_are_named() {
        assert!(validate_request(&[], &options(PageSizing::A4)).is_err());
        let too_many = vec!["a.png".to_string(); MAX_IMAGES + 1];
        assert!(validate_request(&too_many, &options(PageSizing::A4)).is_err());
        let directory = tempfile::tempdir().unwrap();
        let broken = directory.path().join("bozuk.png");
        std::fs::write(&broken, b"not an image").unwrap();
        let error = build_pdf(
            &[broken.to_string_lossy().into_owned()],
            &options(PageSizing::A4),
        )
        .unwrap_err();
        assert!(error.starts_with("bozuk.png:"), "{error}");
    }
}
