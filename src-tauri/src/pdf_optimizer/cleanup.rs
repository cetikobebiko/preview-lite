// Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors.
// Licensed under the MIT License; see src-tauri/licenses/pdfcraft.txt.
//
// Ported from PdfCraft 0.16.1, crates/optimize/src/unused.rs and the clean-up passes of
// crates/optimize/src/lib.rs (MIT OR Apache-2.0, used under MIT). Changes from the original:
// rewritten on lopdf 0.46; encrypted documents never get here (the job refuses them), so the
// encryption cases are gone; no cancellation callback (the job worker is stopped instead).

//! Clean up: unused XObjects, thumbnails, alternate images, streams stored without a filter.
//!
//! Removing an XObject something still draws would make content disappear, so the rule is
//! simple and errs on the side of keeping:
//! - only a page's own resources are cleaned: dictionaries nothing else refers to (resources
//!   shared with other pages, forms or appearances, and inherited ones, are left alone);
//! - a name goes only if it is written nowhere in any stream the document still uses, found by
//!   scanning the bytes wherever the name appears. Nothing can draw a resource by name without
//!   writing the name in a stream, whichever resources a viewer then looks it up in;
//! - if any such stream can't be read, nothing is removed. Only pictures (images in an image
//!   codec, used only as images) are skipped.

use super::walk::name;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::collections::{HashMap, HashSet};

/// Image codecs: a stream encoded with one holds picture data.
const IMAGE_CODECS: [&[u8]; 6] = [
    b"DCTDecode",
    b"DCT",
    b"JPXDecode",
    b"JBIG2Decode",
    b"CCITTFaxDecode",
    b"CCF",
];
/// Most data decoded while looking for names, in all; past it nothing is removed.
const DECODE_BUDGET: usize = 1 << 30;
/// Streams shorter than this don't gain from compression.
const MIN_COMPRESSED_STREAM: usize = 64;

/// Where a page's own `/XObject` dictionary is.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Place {
    /// An object of its own (`/XObject n 0 R`).
    Own(ObjectId),
    /// Directly in the page's resources object (`/Resources n 0 R`).
    InResources(ObjectId),
    /// Directly in resources written directly in the page.
    InPage(ObjectId),
}

/// How an object is used where a reference to it appears.
#[derive(Clone, Copy, PartialEq)]
enum Use {
    /// As a picture: an `/XObject` entry, `/SMask`, `/Mask`, `/Thumb` or an alternate `/Image`.
    Picture,
    /// As an `/XObject` dictionary (its entries are pictures or forms).
    Entries,
    Other,
}

/// The objects the document uses (everything a full rewrite keeps): how many references point at
/// each, and which are referenced other than as a picture.
struct Graph {
    refs: HashMap<ObjectId, usize>,
    not_picture: HashSet<ObjectId>,
}

fn graph<'a>(document: &'a Document) -> Graph {
    let child = |key: &[u8], parent: Use| match (parent, key) {
        (Use::Entries, _) => Use::Picture,
        (_, b"XObject") => Use::Entries,
        (_, b"SMask" | b"Mask" | b"Thumb" | b"Image") => Use::Picture,
        _ => Use::Other,
    };
    let mut graph = Graph {
        refs: HashMap::new(),
        not_picture: HashSet::new(),
    };
    let mut stack: Vec<(&'a Object, Use)> = document
        .trailer
        .iter()
        .map(|(_, value)| (value, Use::Other))
        .collect();
    while let Some((object, how)) = stack.pop() {
        let mut expand = |dict: &'a Dictionary| {
            stack.extend(dict.iter().map(|(key, value)| (value, child(key, how))));
        };
        match object {
            Object::Reference(id) => {
                if how != Use::Picture {
                    graph.not_picture.insert(*id);
                }
                let count = graph.refs.entry(*id).or_default();
                *count += 1;
                if *count > 1 {
                    continue;
                }
                match document.objects.get(id) {
                    Some(Object::Dictionary(dict)) => expand(dict),
                    Some(Object::Stream(stream)) => expand(&stream.dict),
                    Some(Object::Array(values)) => stack.extend(values.iter().map(|v| (v, how))),
                    _ => {}
                }
            }
            Object::Dictionary(dict) => expand(dict),
            Object::Array(values) => stack.extend(values.iter().map(|v| (v, how))),
            _ => {}
        }
    }
    graph
}

/// `page`'s own `/XObject` dictionary, if nothing else refers to it.
fn own_xobjects(
    document: &Document,
    page: ObjectId,
    refs: &HashMap<ObjectId, usize>,
) -> Option<(Place, Dictionary)> {
    let only_once = |id: &ObjectId| refs.get(id) == Some(&1);
    let Some(Object::Dictionary(dict)) = document.objects.get(&page) else {
        return None;
    };
    // A node with kids passes its resources on.
    if name(dict, b"Type") != Some(b"Page") || dict.has(b"Kids") {
        return None;
    }
    let (resources, place) = match dict.get(b"Resources").ok()? {
        Object::Reference(id) if only_once(id) => match document.objects.get(id)? {
            Object::Dictionary(resources) => (resources, Place::InResources(*id)),
            _ => return None,
        },
        Object::Dictionary(resources) => (resources, Place::InPage(page)),
        _ => return None,
    };
    match resources.get(b"XObject").ok()? {
        Object::Reference(id) if only_once(id) => match document.objects.get(id)? {
            Object::Dictionary(xobjects) => Some((Place::Own(*id), xobjects.clone())),
            _ => None,
        },
        Object::Dictionary(xobjects) => Some((place, xobjects.clone())),
        _ => None,
    }
}

fn hex(byte: u8) -> Option<u8> {
    (byte as char).to_digit(16).map(|digit| digit as u8)
}

/// Record which of `wanted` (none longer than `longest`) are written in `data` (`/Name`, `#xx`
/// escapes decoded), wherever they appear: operands, strings, comments or inline image data.
fn find_names(
    data: &[u8],
    wanted: &HashSet<Vec<u8>>,
    longest: usize,
    found: &mut HashSet<Vec<u8>>,
) {
    let regular = |b: u8| {
        !matches!(
            b,
            b'\0'
                | b'\t'
                | b'\n'
                | b'\x0C'
                | b'\r'
                | b' '
                | b'('
                | b')'
                | b'<'
                | b'>'
                | b'['
                | b']'
                | b'{'
                | b'}'
                | b'/'
                | b'%'
        )
    };
    let mut i = 0;
    while let Some(slash) = data
        .get(i..)
        .and_then(|rest| rest.iter().position(|&b| b == b'/'))
    {
        let mut j = i + slash + 1;
        let mut name = Vec::new();
        let mut too_long = false;
        while let Some(&b) = data.get(j) {
            if !regular(b) {
                break;
            }
            let escaped = match (
                b,
                data.get(j + 1).copied().and_then(hex),
                data.get(j + 2).copied().and_then(hex),
            ) {
                (b'#', Some(high), Some(low)) => Some(high << 4 | low),
                _ => None,
            };
            let (byte, step) = escaped.map_or((b, 1), |byte| (byte, 3));
            if name.len() < longest {
                name.push(byte);
            } else {
                too_long = true;
            }
            j += step;
        }
        if !too_long && wanted.contains(&name) {
            found.insert(name);
        }
        i = j;
    }
}

/// What a stream's data is, for finding names in it.
enum Data {
    /// A picture: image data in an image codec, used only as an image.
    Picture,
    Decoded(Vec<u8>),
    /// Not readable the way a viewer would read it.
    Unknown,
}

fn data(stream: &Stream, id: ObjectId, graph: &Graph, limit: usize) -> Data {
    let dict = &stream.dict;
    // External data, or a filter description that is abbreviated or indirect: a viewer might
    // decode it differently from this check.
    if dict.has(b"F")
        || dict.has(b"DP")
        || matches!(dict.get(b"Filter"), Ok(Object::Reference(_)))
        || matches!(dict.get(b"DecodeParms"), Ok(Object::Reference(_)))
    {
        return Data::Unknown;
    }
    let filters: Vec<&[u8]> = match dict.get(b"Filter") {
        Err(_) => Vec::new(),
        Ok(Object::Name(n)) => vec![n.as_slice()],
        Ok(Object::Array(values)) => {
            let names: Vec<&[u8]> = values
                .iter()
                .filter_map(|value| match value {
                    Object::Name(n) => Some(n.as_slice()),
                    _ => None,
                })
                .collect();
            if names.len() != values.len() {
                return Data::Unknown;
            }
            names
        }
        Ok(_) => return Data::Unknown,
    };
    if filters.contains(&&b"Crypt"[..]) {
        return Data::Unknown;
    }
    if filters.iter().any(|filter| IMAGE_CODECS.contains(filter)) {
        // Viewers decode any stream's codec, so picture data skipped here must be a picture.
        let picture = name(dict, b"Subtype") == Some(b"Image") && !graph.not_picture.contains(&id);
        return if picture {
            Data::Picture
        } else {
            Data::Unknown
        };
    }
    // Parameters only as direct integers, which this check and viewers read alike.
    let direct = |parms: &Object| match parms {
        Object::Dictionary(parms) => parms
            .iter()
            .all(|(_, value)| matches!(value, Object::Integer(_))),
        Object::Null => true,
        _ => false,
    };
    let parms_line_up = match dict.get(b"DecodeParms") {
        Err(_) => true,
        Ok(parms @ (Object::Dictionary(_) | Object::Null)) => filters.len() == 1 && direct(parms),
        Ok(Object::Array(values)) => values.len() == filters.len() && values.iter().all(direct),
        Ok(_) => false,
    };
    if !parms_line_up {
        return Data::Unknown;
    }
    stream
        .decompressed_content_with_limit(limit)
        .map_or(Data::Unknown, Data::Decoded)
}

/// Drop the image and form XObjects that `pages` list in their own resources but no stream
/// names. Returns how many went.
pub(super) fn remove_unused(document: &mut Document, pages: &[ObjectId]) -> usize {
    let graph = graph(document);
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for page in pages {
        let Some((place, xobjects)) = own_xobjects(document, *page, &graph.refs) else {
            continue;
        };
        if !seen.insert(place) {
            continue;
        }
        let names: Vec<Vec<u8>> = xobjects
            .iter()
            .filter(|(_, value)| {
                let target = match *value {
                    Object::Reference(id) => document.objects.get(id),
                    other => Some(other),
                };
                matches!(target, Some(Object::Stream(stream))
                    if matches!(name(&stream.dict, b"Subtype"), Some(b"Image" | b"Form")))
            })
            .map(|(key, _)| key.clone())
            .collect();
        if !names.is_empty() {
            candidates.push((place, names));
        }
    }
    let wanted: HashSet<Vec<u8>> = candidates
        .iter()
        .flat_map(|(_, names)| names.iter().cloned())
        .collect();
    let longest = wanted.iter().map(Vec::len).max().unwrap_or(0);
    let mut written = HashSet::new();
    let mut decoded = 0usize;
    for id in graph.refs.keys() {
        if written.len() == wanted.len() {
            return 0;
        }
        let Some(Object::Stream(stream)) = document.objects.get(id) else {
            continue;
        };
        match data(stream, *id, &graph, DECODE_BUDGET.saturating_sub(decoded)) {
            Data::Picture => {}
            Data::Decoded(bytes) => {
                decoded = decoded.saturating_add(bytes.len());
                find_names(&bytes, &wanted, longest, &mut written);
            }
            Data::Unknown => return 0,
        }
    }
    let mut removed = 0;
    for (place, names) in candidates {
        let gone: Vec<Vec<u8>> = names
            .into_iter()
            .filter(|name| !written.contains(name))
            .collect();
        if gone.is_empty() {
            continue;
        }
        let xobjects = match place {
            Place::Own(id) => match document.objects.get_mut(&id) {
                Some(Object::Dictionary(xobjects)) => Some(xobjects),
                _ => None,
            },
            Place::InResources(id) => match document.objects.get_mut(&id) {
                Some(Object::Dictionary(resources)) => match resources.get_mut(b"XObject") {
                    Ok(Object::Dictionary(xobjects)) => Some(xobjects),
                    _ => None,
                },
                _ => None,
            },
            Place::InPage(id) => match document.objects.get_mut(&id) {
                Some(Object::Dictionary(page)) => match page.get_mut(b"Resources") {
                    Ok(Object::Dictionary(resources)) => match resources.get_mut(b"XObject") {
                        Ok(Object::Dictionary(xobjects)) => Some(xobjects),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            },
        };
        if let Some(xobjects) = xobjects {
            for name in &gone {
                if xobjects.remove(name).is_some() {
                    removed += 1;
                }
            }
        }
    }
    removed
}

/// Removes the pages' `/Thumb` entries. Returns how many went.
pub(super) fn discard_thumbnails(document: &mut Document, pages: &[ObjectId]) -> usize {
    pages
        .iter()
        .filter(|page| match document.objects.get_mut(page) {
            Some(Object::Dictionary(dict)) => dict.remove(b"Thumb").is_some(),
            _ => false,
        })
        .count()
}

/// Removes images' `/Alternates`. Returns how many images had some.
pub(super) fn discard_alternate_images(document: &mut Document) -> usize {
    let mut discarded = 0;
    for object in document.objects.values_mut() {
        if let Object::Stream(stream) = object {
            if name(&stream.dict, b"Subtype") == Some(b"Image")
                && stream.dict.remove(b"Alternates").is_some()
            {
                discarded += 1;
            }
        }
    }
    discarded
}

/// Flate-compresses streams stored without a filter, when that is smaller. XMP stays readable
/// as plain text (PDF/A requires it). Returns how many were compressed.
pub(super) fn flate_unencoded(document: &mut Document) -> usize {
    let mut compressed = 0;
    for object in document.objects.values_mut() {
        let Object::Stream(stream) = object else {
            continue;
        };
        if stream.dict.has(b"Filter")
            || stream.dict.has(b"F")
            || matches!(
                name(&stream.dict, b"Type"),
                Some(b"Metadata" | b"XRef" | b"ObjStm")
            )
            || stream.content.len() < MIN_COMPRESSED_STREAM
        {
            continue;
        }
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(&stream.content, 9);
        if packed.len() + 19 < stream.content.len() {
            stream.dict.remove(b"DecodeParms");
            stream
                .dict
                .set("Filter", Object::Name(b"FlateDecode".to_vec()));
            stream.set_content(packed);
            compressed += 1;
        }
    }
    compressed
}
