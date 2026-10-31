// Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors.
// Licensed under the MIT License; see src-tauri/licenses/pdfcraft.txt.
//
// Ported from PdfCraft 0.16.1, crates/optimize/src/images.rs (MIT OR Apache-2.0, used under MIT).
// Changes from the original: rewritten on lopdf 0.46; JPEGs are decoded with the `image` crate
// (no direct zune-jpeg dependency), so only the JPEGs it and PDF readers read alike are taken:
// grey ones, and three-component YCbCr ones without RGB component names; filter parameters given
// by reference are not resolved (such images are left alone); black-and-white images, image
// fragments and default colour spaces are not handled.

//! Image resampling and recompression.

use super::walk::{effective_resolutions, int, number, resolve, used_elsewhere};
use super::{ImageCompression, ImageSettings, OptimizeReport, OptimizeSettings};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::collections::HashSet;

/// Images with more pixels than this are left alone (memory).
const MAX_PIXELS: u64 = 64_000_000;
/// Largest JPEG decoded out of the generic filters around it.
const MAX_JPEG_BYTES: usize = 256 * 1024 * 1024;
/// Markers and fill bytes read before the first scan of a JPEG, at most.
const MAX_HEADER_STEPS: usize = 4096;
/// Images smaller than this aren't recompressed without resampling: icons and rules don't gain,
/// and JPEG would blur them.
const MIN_RECOMPRESS_PIXELS: u64 = 64 * 64;

/// Generic filters whose output this module reads (everything but image codecs).
const GENERIC_FILTERS: [&[u8]; 10] = [
    b"FlateDecode",
    b"Fl",
    b"LZWDecode",
    b"LZW",
    b"RunLengthDecode",
    b"RL",
    b"ASCII85Decode",
    b"A85",
    b"ASCIIHexDecode",
    b"AHx",
];

/// Colour components of a colour space resampled here: grey (1) or RGB (3).
pub(crate) fn components(document: &Document, colour_space: &Object) -> Option<usize> {
    match resolve(document, colour_space) {
        Object::Name(n) => match n.as_slice() {
            b"DeviceGray" | b"G" => Some(1),
            b"DeviceRGB" | b"RGB" => Some(3),
            _ => None,
        },
        Object::Array(values) => match resolve(document, values.first()?) {
            Object::Name(family) => match family.as_slice() {
                b"ICCBased" => match resolve(document, values.get(1)?) {
                    Object::Stream(icc) => match int(&icc.dict, b"N")? {
                        1 => Some(1),
                        3 => Some(3),
                        _ => None,
                    },
                    _ => None,
                },
                b"CalGray" => Some(1),
                b"CalRGB" => Some(3),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// The stream's filter names; empty without a filter, `None` if they can't be read.
fn filter_names(stream: &Stream) -> Option<Vec<&[u8]>> {
    Some(match stream.dict.get(b"Filter") {
        Err(_) => Vec::new(),
        Ok(Object::Name(n)) => vec![n.as_slice()],
        Ok(Object::Array(values)) => values
            .iter()
            .map(|value| match value {
                Object::Name(n) => Some(n.as_slice()),
                _ => None,
            })
            .collect::<Option<_>>()?,
        Ok(_) => return None,
    })
}

/// The stream's codec: `Some(None)` for data under generic filters only, `Some(Some(k))` for a
/// JPEG under `k` generic filters (scanners write `[/FlateDecode /DCTDecode]`), `None` for
/// anything else (another codec, a filter given by reference).
fn jpeg_layers(stream: &Stream) -> Option<Option<usize>> {
    let names = filter_names(stream)?;
    let generic = |filter: &&[u8]| GENERIC_FILTERS.contains(filter);
    match names.split_last() {
        Some((last, before))
            if matches!(*last, b"DCTDecode" | b"DCT") && before.iter().all(generic) =>
        {
            Some(Some(before.len()))
        }
        _ if names.iter().all(generic) => Some(None),
        _ => None,
    }
}

/// The JPEG data of a stream whose JPEG is under `layers` generic filters, those decoded.
fn jpeg_data(stream: &Stream, layers: usize) -> Option<std::borrow::Cow<'_, [u8]>> {
    if layers == 0 {
        return Some(std::borrow::Cow::Borrowed(&stream.content));
    }
    let names = filter_names(stream)?;
    let mut dict = Dictionary::new();
    let outer: Vec<Object> = names[..layers]
        .iter()
        .map(|name| Object::Name(name.to_vec()))
        .collect();
    dict.set("Filter", Object::Array(outer));
    // One entry per filter, as §7.4 pairs them; a single dictionary would reach every filter.
    match stream.dict.get(b"DecodeParms") {
        Err(_) | Ok(Object::Null) => {}
        Ok(Object::Array(values))
            if values.len() == names.len()
                && values
                    .iter()
                    .all(|value| matches!(value, Object::Null | Object::Dictionary(_))) =>
        {
            dict.set("DecodeParms", Object::Array(values[..layers].to_vec()));
        }
        Ok(_) => return None,
    }
    let outer = Stream::new(dict, stream.content.clone());
    outer
        .decompressed_content_with_limit(MAX_JPEG_BYTES)
        .ok()
        .map(std::borrow::Cow::Owned)
}

/// Whether the filter parameters are written directly, as lopdf reads them: it takes a
/// reference as no parameters, and a predictor's tag bytes would then shift every row.
fn direct_parms(dict: &Dictionary) -> bool {
    match dict.get(b"DecodeParms") {
        Err(_) | Ok(Object::Null) | Ok(Object::Dictionary(_)) => true,
        Ok(Object::Array(values)) => values
            .iter()
            .all(|value| matches!(value, Object::Null | Object::Dictionary(_))),
        Ok(_) => false,
    }
}

/// What a JPEG's header says before its first scan.
pub(super) struct JpegHeader {
    /// The transform flag of its last Adobe APP14 segment.
    adobe: Option<u8>,
    /// Component identifiers of its frame.
    components: Vec<u8>,
    width: usize,
    height: usize,
}

/// Reads a JPEG's segments up to the first scan. `None` if they can't be read.
pub(super) fn jpeg_header(data: &[u8]) -> Option<JpegHeader> {
    if data.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut header = JpegHeader {
        adobe: None,
        components: Vec::new(),
        width: 0,
        height: 0,
    };
    let mut i = 2usize;
    for _ in 0..MAX_HEADER_STEPS {
        if *data.get(i)? != 0xFF {
            return None;
        }
        let marker = *data.get(i.checked_add(1)?)?;
        match marker {
            0xFF => i = i.checked_add(1)?,
            0xDA | 0xD9 => return (!header.components.is_empty()).then_some(header),
            0x01 | 0xD0..=0xD8 => i = i.checked_add(2)?,
            _ => {
                let len = usize::from(u16::from_be_bytes([
                    *data.get(i.checked_add(2)?)?,
                    *data.get(i.checked_add(3)?)?,
                ]));
                let end = i.checked_add(2)?.checked_add(len)?;
                let segment = data.get(i.checked_add(4)?..end)?;
                match marker {
                    0xEE => {
                        if let Some(rest) = segment.strip_prefix(b"Adobe") {
                            header.adobe = Some(*rest.get(6)?);
                        }
                    }
                    // Start of frame, every kind but the DHT/JPG/DAC markers in that range.
                    0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                        header.height =
                            usize::from(u16::from_be_bytes([*segment.get(1)?, *segment.get(2)?]));
                        header.width =
                            usize::from(u16::from_be_bytes([*segment.get(3)?, *segment.get(4)?]));
                        let count = usize::from(*segment.get(5)?);
                        header.components = (0..count)
                            .map(|k| segment.get(6 + 3 * k).copied())
                            .collect::<Option<_>>()?;
                    }
                    _ => {}
                }
                i = end;
            }
        }
    }
    None
}

/// The `/ColorTransform` of a JPEG's filter parameters (the JPEG is the last filter);
/// `Some(None)` without one, `None` if it can't be read.
fn color_transform(stream: &Stream) -> Option<Option<i64>> {
    let filters = filter_names(stream)?.len();
    let parms = match stream.dict.get(b"DecodeParms") {
        Err(_) | Ok(Object::Null) => return Some(None),
        Ok(Object::Dictionary(dict)) if filters == 1 => dict,
        Ok(Object::Array(values)) if values.len() == filters => match values.last()? {
            Object::Null => return Some(None),
            Object::Dictionary(dict) => dict,
            _ => return None,
        },
        Ok(_) => return None,
    };
    match parms.get(b"ColorTransform") {
        Err(_) => Some(None),
        Ok(Object::Integer(value)) => Some(Some(*value)),
        Ok(_) => None,
    }
}

/// Whether the `image` crate decodes this JPEG to the samples a PDF reader draws. Grey JPEGs
/// always; three-component ones only as YCbCr (ISO 32000-2 §7.4.8: an Adobe marker of 1, or no
/// marker and `/ColorTransform` 1 or absent) with component ids other than R, G, B, which the
/// decoder would take as stored RGB. Other combinations are rare, and readers disagree on them.
pub(super) fn jpeg_reads_alike(stream: &Stream, n: usize, header: &JpegHeader) -> bool {
    if header.components.len() != n {
        return false;
    }
    if n == 1 {
        return true;
    }
    let Some(entry) = color_transform(stream) else {
        return false;
    };
    let ycc = match header.adobe {
        Some(1) => entry.is_none() || entry == Some(1),
        Some(_) => false,
        None => entry.is_none() || entry == Some(1),
    };
    ycc && header.components.as_slice() != b"RGB"
}

fn jpeg_samples(data: &[u8], w: u32, h: u32, n: usize) -> Option<Vec<u8>> {
    use image::ImageDecoder;
    let decoder = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(data)).ok()?;
    if decoder.dimensions() != (w, h) {
        return None;
    }
    let expected = match n {
        1 => image::ColorType::L8,
        _ => image::ColorType::Rgb8,
    };
    if decoder.color_type() != expected {
        return None;
    }
    let mut out = vec![0; usize::try_from(decoder.total_bytes()).ok()?];
    decoder.read_image(&mut out).ok()?;
    Some(out)
}

/// How far an image of `bytes` samples in `rows` rows is decoded: more can only be a
/// decompression bomb, and the image is left alone.
fn allowance(bytes: usize, rows: u32) -> Option<usize> {
    bytes
        .checked_add(usize::try_from(rows).ok()?)?
        .checked_mul(3)?
        .checked_add(4096)
}

/// 8-bit samples of an image with `n` components, decoded. `None` for a JPEG this module doesn't
/// read the way readers do, and for data that doesn't decode to the samples the size needs.
fn pixels(stream: &Stream, n: usize) -> Option<Vec<u8>> {
    let w = u32::try_from(int(&stream.dict, b"Width")?).ok()?;
    let h = u32::try_from(int(&stream.dict, b"Height")?).ok()?;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return None;
    }
    let bytes = usize::try_from(w)
        .ok()?
        .checked_mul(usize::try_from(h).ok()?)?
        .checked_mul(n)?;
    if let Some(layers) = jpeg_layers(stream)? {
        let data = jpeg_data(stream, layers)?;
        let header = jpeg_header(&data)?;
        if (header.width, header.height) != (w as usize, h as usize)
            || !jpeg_reads_alike(stream, n, &header)
        {
            return None;
        }
        let samples = jpeg_samples(&data, w, h, n)?;
        (samples.len() == bytes).then_some(samples)
    } else {
        if !direct_parms(&stream.dict) {
            return None;
        }
        let samples = stream
            .decompressed_content_with_limit(allowance(bytes, h)?)
            .ok()?;
        Some(samples.get(..bytes)?.to_vec())
    }
}

fn resample(px: &[u8], n: usize, w: u32, h: u32, nw: u32, nh: u32) -> Option<Vec<u8>> {
    use image::imageops::{resize, FilterType};
    Some(match n {
        1 => resize(
            &image::GrayImage::from_raw(w, h, px.to_vec())?,
            nw,
            nh,
            FilterType::CatmullRom,
        )
        .into_raw(),
        _ => resize(
            &image::RgbImage::from_raw(w, h, px.to_vec())?,
            nw,
            nh,
            FilterType::CatmullRom,
        )
        .into_raw(),
    })
}

fn jpeg(px: &[u8], n: usize, w: u32, h: u32, quality: u8) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100));
    let colour = if n == 1 {
        image::ExtendedColorType::L8
    } else {
        image::ExtendedColorType::Rgb8
    };
    encoder.encode(px, w, h, colour).ok()?;
    Some(out)
}

/// Whether `decode` is the default decode array for `n` components (`[0 1]` each).
fn is_identity_decode(document: &Document, decode: &Object, n: usize) -> bool {
    let Object::Array(values) = resolve(document, decode) else {
        return false;
    };
    values.len() == 2 * n
        && values.chunks(2).all(|pair| {
            number(resolve(document, &pair[0])) == Some(0.0)
                && number(resolve(document, &pair[1])) == Some(1.0)
        })
}

/// A new image stream from 8-bit samples: the original dictionary with the new size and filter.
/// The decode array (only the default one gets here) and filter parameters are dropped: the
/// samples are decoded, and a JPEG written here is plain YCbCr, which no `/ColorTransform` means.
fn image_stream(
    old: &Dictionary,
    px: &[u8],
    n: usize,
    (w, h): (u32, u32),
    compression: ImageCompression,
    was_dct: bool,
) -> Option<Stream> {
    let mut dict = old.clone();
    for key in [&b"Filter"[..], b"DecodeParms", b"Decode", b"Length", b"DL"] {
        dict.remove(key);
    }
    dict.set("Width", i64::from(w));
    dict.set("Height", i64::from(h));
    dict.set("BitsPerComponent", 8);
    let quality = match compression {
        ImageCompression::Jpeg { quality } => Some(quality),
        ImageCompression::Flate => None,
        ImageCompression::Retain => was_dct.then_some(80),
    };
    let content = match quality {
        Some(quality) => {
            dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
            jpeg(px, n, w, h, quality)?
        }
        None => {
            dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
            miniz_oxide::deflate::compress_to_vec_zlib(px, 6)
        }
    };
    Some(Stream::new(dict, content))
}

/// A replacement image and, when it was resampled with it, its soft mask.
struct Processed {
    image: Stream,
    smask: Option<(ObjectId, Stream)>,
}

/// One image: resample and/or recompress per `settings`; `ppi` is its effective resolution.
/// `None` unless the result is smaller.
fn process(
    document: &Document,
    stream: &Stream,
    ppi: f64,
    settings: &ImageSettings,
    n: usize,
) -> Option<Processed> {
    let dict = &stream.dict;
    if dict
        .get(b"Decode")
        .is_ok_and(|decode| !is_identity_decode(document, decode, n))
        || dict.has(b"Mask")
        || matches!(dict.get(b"ImageMask"), Ok(Object::Boolean(true)))
        || int(dict, b"BitsPerComponent") != Some(8)
    {
        return None;
    }
    let was_dct = jpeg_layers(stream)?.is_some();
    let w = u32::try_from(int(dict, b"Width")?).ok()?;
    let h = u32::try_from(int(dict, b"Height")?).ok()?;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return None;
    }
    let resize = settings.downsample && ppi.is_finite() && ppi > settings.above_ppi;
    let recompress = matches!(settings.compression, ImageCompression::Jpeg { .. })
        || (settings.compression == ImageCompression::Flate && was_dct);
    if !resize && (!recompress || u64::from(w) * u64::from(h) < MIN_RECOMPRESS_PIXELS) {
        return None;
    }
    let (nw, nh) = if resize {
        let k = settings.target_ppi / ppi;
        (
            ((f64::from(w) * k).round() as u32).max(1),
            ((f64::from(h) * k).round() as u32).max(1),
        )
    } else {
        (w, h)
    };
    let px = pixels(stream, n)?;
    let px = if (nw, nh) != (w, h) {
        resample(&px, n, w, h, nw, nh)?
    } else {
        px
    };
    let image = image_stream(dict, &px, n, (nw, nh), settings.compression, was_dct)?;
    // A soft mask is resampled with its image (losslessly), so the two keep the same size.
    let mut smask = None;
    let mut before = stream.content.len();
    if let Ok(Object::Reference(mask_id)) = dict.get(b"SMask") {
        let Some(Object::Stream(mask)) = document.objects.get(mask_id) else {
            return None;
        };
        before += mask.content.len();
        if (nw, nh) != (w, h) {
            if int(&mask.dict, b"BitsPerComponent") != Some(8)
                || mask.dict.has(b"Matte")
                || mask.dict.has(b"Decode")
            {
                return None;
            }
            let mw = u32::try_from(int(&mask.dict, b"Width")?).ok()?;
            let mh = u32::try_from(int(&mask.dict, b"Height")?).ok()?;
            let samples = pixels(mask, 1)?;
            let samples = resample(&samples, 1, mw, mh, nw, nh)?;
            let new_mask = image_stream(
                &mask.dict,
                &samples,
                1,
                (nw, nh),
                ImageCompression::Flate,
                false,
            )?;
            smask = Some((*mask_id, new_mask));
        }
    }
    let after = image.content.len()
        + smask.as_ref().map_or(0, |(_, mask)| mask.content.len())
        + match (&smask, dict.get(b"SMask")) {
            (None, Ok(Object::Reference(mask_id))) => match document.objects.get(mask_id) {
                Some(Object::Stream(mask)) => mask.content.len(),
                _ => 0,
            },
            _ => 0,
        };
    (after < before).then_some(Processed { image, smask })
}

/// Resample and recompress the colour and grey images `pages` draw.
pub(super) fn run(
    document: &mut Document,
    pages: &[ObjectId],
    settings: &OptimizeSettings,
    report: &mut OptimizeReport,
) {
    let resolutions = effective_resolutions(document, pages);
    let images: HashSet<ObjectId> = resolutions.ppi.keys().copied().collect();
    // An image drawn somewhere the walk didn't look (an annotation, a pattern) may be drawn
    // larger there: it may be recompressed, but not resampled.
    let elsewhere = used_elsewhere(document, &images, &resolutions.read);
    // Soft masks are processed with their image, not on their own; one shared by several images
    // (drawn by the pages or not), or drawn as an image too, would get two sizes, so those images
    // keep theirs.
    let mut mask_users: std::collections::HashMap<ObjectId, usize> = Default::default();
    for object in document.objects.values() {
        if let Object::Stream(stream) = object {
            if let Ok(Object::Reference(mask)) = stream.dict.get(b"SMask") {
                *mask_users.entry(*mask).or_default() += 1;
            }
        }
    }
    let mut ids: Vec<(ObjectId, f64)> = resolutions
        .ppi
        .into_iter()
        .filter(|(id, _)| !mask_users.contains_key(id))
        .collect();
    ids.sort_by_key(|(id, _)| *id);
    for (id, ppi) in ids {
        let Some(Object::Stream(stream)) = document.objects.get(&id) else {
            continue;
        };
        let Some(n) = stream
            .dict
            .get(b"ColorSpace")
            .ok()
            .and_then(|cs| components(document, cs))
        else {
            continue;
        };
        report.images += 1;
        let shared_mask = match stream.dict.get(b"SMask") {
            Ok(Object::Reference(mask)) => {
                mask_users.get(mask).copied().unwrap_or(0) > 1 || images.contains(mask)
            }
            _ => false,
        };
        let ppi = if elsewhere.contains(&id) || shared_mask {
            f64::INFINITY
        } else {
            ppi
        };
        let image_settings = if n == 1 {
            &settings.gray
        } else {
            &settings.color
        };
        let Some(processed) = process(document, stream, ppi, image_settings, n) else {
            continue;
        };
        let resized = int(&processed.image.dict, b"Width") != int(&stream.dict, b"Width")
            || int(&processed.image.dict, b"Height") != int(&stream.dict, b"Height");
        report.image_bytes_before += stream.content.len() as u64;
        report.image_bytes_after += processed.image.content.len() as u64;
        if resized {
            report.images_resampled += 1;
        } else {
            report.images_recompressed += 1;
        }
        if let Some((mask_id, mask)) = processed.smask {
            document.objects.insert(mask_id, Object::Stream(mask));
        }
        document.objects.insert(id, Object::Stream(processed.image));
    }
}
