// Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors.
// Licensed under the MIT License; see src-tauri/licenses/pdfcraft.txt.
//
// Ported from PdfCraft 0.16.1, crates/optimize/src/images.rs (the effective-resolution walk and
// `used_elsewhere`; MIT OR Apache-2.0, used under MIT). Changes from the original: rewritten on
// lopdf 0.46 (object ids instead of object numbers, lopdf's content parser); default colour
// spaces are not tracked (they only matter to black-and-white packing, which is not ported); an
// image drawn beyond what the walk read is never resampled here, where PdfCraft only kept it from
// being packed to one bit.

//! Where the pages draw each image, and at what resolution.

use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::{HashMap, HashSet};

/// Forms nested deeper than this aren't followed.
const MAX_FORM_DEPTH: usize = 12;
/// Walks of a form already walked once (other scale or caller) before the measurement gives up.
const MAX_REWALKS: usize = 100_000;
/// Decoded content bytes read for one page, its forms included.
const PAGE_DECODE_BUDGET: usize = 256 * 1024 * 1024;
/// Page-tree levels followed upwards.
const MAX_TREE_DEPTH: usize = 64;

/// A transformation matrix `[a b c d e f]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Matrix(pub(crate) [f64; 6]);

impl Matrix {
    pub(crate) const IDENTITY: Matrix = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    fn from_operands(operands: &[Object]) -> Option<Matrix> {
        if operands.len() != 6 {
            return None;
        }
        let mut m = [0.0; 6];
        for (slot, operand) in m.iter_mut().zip(operands) {
            *slot = number(operand)?;
        }
        m.iter().all(|v| v.is_finite()).then_some(Matrix(m))
    }

    /// `self` applied first, then `outer`.
    pub(crate) fn then(&self, outer: &Matrix) -> Matrix {
        let [a, b, c, d, e, f] = self.0;
        let [oa, ob, oc, od, oe, of] = outer.0;
        Matrix([
            a * oa + b * oc,
            a * ob + b * od,
            c * oa + d * oc,
            c * ob + d * od,
            e * oa + f * oc + oe,
            e * ob + f * od + of,
        ])
    }
}

pub(crate) fn number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some(f64::from(*value)),
        _ => None,
    }
}

/// `object`, or the object it refers to (references followed a few steps).
pub(crate) fn resolve<'a>(document: &'a Document, object: &'a Object) -> &'a Object {
    let mut current = object;
    for _ in 0..8 {
        match current {
            Object::Reference(id) => match document.objects.get(id) {
                Some(next) => current = next,
                None => return &Object::Null,
            },
            _ => return current,
        }
    }
    &Object::Null
}

pub(crate) fn dict_of<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match resolve(document, object) {
        Object::Dictionary(dict) => Some(dict),
        Object::Stream(stream) => Some(&stream.dict),
        _ => None,
    }
}

pub(crate) fn name<'a>(dict: &'a Dictionary, key: &[u8]) -> Option<&'a [u8]> {
    match dict.get(key) {
        Ok(Object::Name(name)) => Some(name),
        _ => None,
    }
}

pub(crate) fn int(dict: &Dictionary, key: &[u8]) -> Option<i64> {
    match dict.get(key) {
        Ok(Object::Integer(value)) => Some(*value),
        _ => None,
    }
}

/// Where a walk read a reference: under a name in an `/XObject` dictionary (an image or a form),
/// as resources' `/XObject` dictionary, or as the `/Resources` of a page, node or form.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Entry {
    Name(Vec<u8>),
    XObject,
    Resources,
}

/// The references a walk followed: (the object referred to, the object holding the reference,
/// where in it), and the pages and page-tree nodes whose resources it read.
#[derive(Default)]
pub(crate) struct Read {
    entries: HashSet<(ObjectId, ObjectId, Entry)>,
    roots: HashSet<ObjectId>,
}

/// What a walk over the pages found.
pub(crate) struct Resolutions {
    /// The effective resolution (ppi) of every image the pages draw: the smallest over its uses,
    /// `INFINITY` when a use may have been missed.
    pub(crate) ppi: HashMap<ObjectId, f64>,
    pub(crate) read: Read,
}

struct Walk<'a> {
    document: &'a Document,
    /// The forms being walked right now: a form drawing one of these is a cycle.
    path: Vec<ObjectId>,
    /// Forms with their own resources already walked, by the linear part of their matrix.
    walked: HashSet<(ObjectId, [u64; 4])>,
    seen: HashSet<ObjectId>,
    rewalks: usize,
    /// Decoded bytes left for the page being walked.
    budget: usize,
    /// False once a use may have been missed.
    complete: bool,
    out: HashMap<ObjectId, f64>,
    read: Read,
}

impl Walk<'_> {
    /// Decodes a stream within what is left of the page's budget.
    fn decode(&mut self, stream: &lopdf::Stream) -> Option<Vec<u8>> {
        let data = stream.decompressed_content_with_limit(self.budget).ok()?;
        self.budget = self.budget.saturating_sub(data.len());
        Some(data)
    }

    /// Walk one content stream, recording each image's displayed resolution (pixels per inch,
    /// the smaller of the two axes) and descending into form XObjects. `holder` is the object
    /// holding `resources`.
    fn content(&mut self, data: &[u8], resources: &Dictionary, holder: ObjectId, ctm: Matrix) {
        let document = self.document;
        let xobjects = resources
            .get(b"XObject")
            .ok()
            .and_then(|x| dict_of(document, x))
            .cloned()
            .unwrap_or_default();
        let xholder = match resources.get(b"XObject") {
            Ok(Object::Reference(x)) => {
                self.read.entries.insert((*x, holder, Entry::XObject));
                *x
            }
            _ => holder,
        };
        // Every entry counts as read, drawn here or not: content using these resources (a Type3
        // glyph without its own, say) draws it the same way.
        for (key, value) in xobjects.iter() {
            if let Object::Reference(id) = value {
                self.read
                    .entries
                    .insert((*id, xholder, Entry::Name(key.clone())));
            }
        }
        if data.is_empty() {
            return;
        }
        // Strict: lopdf's lenient decoder drops whatever follows a token it can't read, where a
        // viewer steps over the token and draws the rest, so later uses would go unseen.
        let Ok(content) = Content::decode_strict(data) else {
            self.complete = false;
            return;
        };
        let mut stack = vec![ctm];
        for operation in content.operations {
            let top = stack.last().copied().unwrap_or(ctm);
            match operation.operator.as_str() {
                "q" => stack.push(top),
                "Q" if stack.len() > 1 => {
                    stack.pop();
                }
                "cm" => {
                    if let Some(m) = Matrix::from_operands(&operation.operands) {
                        if let Some(current) = stack.last_mut() {
                            *current = m.then(&top);
                        }
                    }
                }
                "Do" => {
                    let Some(Object::Name(key)) = operation.operands.first() else {
                        continue;
                    };
                    let Ok(Object::Reference(id)) = xobjects.get(key) else {
                        continue;
                    };
                    let Some(Object::Stream(stream)) = document.objects.get(id) else {
                        continue;
                    };
                    match name(&stream.dict, b"Subtype") {
                        Some(b"Image") => {
                            let width = int(&stream.dict, b"Width").unwrap_or(0) as f64;
                            let height = int(&stream.dict, b"Height").unwrap_or(0) as f64;
                            let [a, b, c, d, ..] = top.0;
                            let (sx, sy) = ((a * a + b * b).sqrt(), (c * c + d * d).sqrt());
                            if width > 0.0 && height > 0.0 && sx > 1e-6 && sy > 1e-6 {
                                let ppi = (width / (sx / 72.0)).min(height / (sy / 72.0));
                                let entry = self.out.entry(*id).or_insert(f64::INFINITY);
                                *entry = entry.min(ppi);
                            }
                        }
                        Some(b"Form") => self.form(*id, stream, (resources, holder), top),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    /// Read the resources of page `page` and of each page-tree node above it, each of them a
    /// root: a node's resources are its inheriting pages' (every page is walked), or no page's.
    fn resources_above(&mut self, page: ObjectId) {
        let document = self.document;
        let mut node = Some(page);
        for _ in 0..MAX_TREE_DEPTH {
            let Some(id) = node else { return };
            // A node met before had its own nodes above it read then.
            if !self.read.roots.insert(id) {
                return;
            }
            let Some(Object::Dictionary(dict)) = document.objects.get(&id) else {
                return;
            };
            if let Ok(value) = dict.get(b"Resources") {
                if let Some(resources) = dict_of(document, value) {
                    let holder = match value {
                        Object::Reference(res) => {
                            self.read.entries.insert((*res, id, Entry::Resources));
                            *res
                        }
                        _ => id,
                    };
                    let resources = resources.clone();
                    self.content(&[], &resources, holder, Matrix::IDENTITY);
                }
            }
            node = match dict.get(b"Parent") {
                Ok(Object::Reference(parent)) => Some(*parent),
                _ => None,
            };
        }
    }

    /// Descend into form `id` drawn with `ctm` from a stream whose resources are `resources`,
    /// with the object holding them.
    fn form(
        &mut self,
        id: ObjectId,
        stream: &lopdf::Stream,
        (resources, holder): (&Dictionary, ObjectId),
        ctm: Matrix,
    ) {
        if self.path.contains(&id) {
            return;
        }
        if self.path.len() >= MAX_FORM_DEPTH {
            self.complete = false;
            return;
        }
        let document = self.document;
        let matrix = stream
            .dict
            .get(b"Matrix")
            .ok()
            .and_then(|m| match resolve(document, m) {
                Object::Array(values) => Matrix::from_operands(values),
                _ => None,
            });
        let inner = matrix.unwrap_or(Matrix::IDENTITY).then(&ctm);
        let own = stream
            .dict
            .get(b"Resources")
            .ok()
            .and_then(|r| dict_of(document, r))
            .cloned();
        // A form without resources uses its caller's, so the same form can draw other images.
        if own.is_some() {
            let [a, b, c, d, ..] = inner.0;
            if !self
                .walked
                .insert((id, [a.to_bits(), b.to_bits(), c.to_bits(), d.to_bits()]))
            {
                return;
            }
        }
        if !self.seen.insert(id) {
            if self.rewalks >= MAX_REWALKS {
                self.complete = false;
                return;
            }
            self.rewalks += 1;
        }
        let Some(data) = self.decode(stream) else {
            self.complete = false;
            return;
        };
        self.path.push(id);
        match &own {
            Some(own) => {
                let own_holder = match stream.dict.get(b"Resources") {
                    Ok(Object::Reference(res)) => {
                        self.read.entries.insert((*res, id, Entry::Resources));
                        *res
                    }
                    _ => id,
                };
                self.content(&data, own, own_holder, inner);
            }
            None => self.content(&data, resources, holder, inner),
        }
        self.path.pop();
    }
}

/// The resources of a page, inherited from the page tree if the page has none, and the object
/// holding them: the page or node they are written in, or the resources dictionary when it is an
/// object of its own.
fn held_page_resources(document: &Document, page: ObjectId) -> Option<(Dictionary, ObjectId)> {
    let mut node = Some(page);
    for _ in 0..MAX_TREE_DEPTH {
        let id = node?;
        let Some(Object::Dictionary(dict)) = document.objects.get(&id) else {
            return None;
        };
        if let Ok(value) = dict.get(b"Resources") {
            let holder = match value {
                Object::Reference(res) => *res,
                _ => id,
            };
            return Some((dict_of(document, value)?.clone(), holder));
        }
        node = match dict.get(b"Parent") {
            Ok(Object::Reference(parent)) => Some(*parent),
            _ => None,
        };
    }
    None
}

/// Inherited `/UserUnit`, 1 by default: content is in page units, resolution in physical inches.
fn user_unit(document: &Document, page: ObjectId) -> f64 {
    let Some(Object::Dictionary(dict)) = document.objects.get(&page) else {
        return 1.0;
    };
    dict.get(b"UserUnit")
        .ok()
        .and_then(|unit| number(resolve(document, unit)))
        .filter(|unit| unit.is_finite() && *unit > 0.0)
        .unwrap_or(1.0)
}

/// The effective resolution of every image `pages` draw (see [`Resolutions`]).
pub(crate) fn effective_resolutions(document: &Document, pages: &[ObjectId]) -> Resolutions {
    let mut walk = Walk {
        document,
        path: Vec::new(),
        walked: HashSet::new(),
        seen: HashSet::new(),
        rewalks: 0,
        budget: PAGE_DECODE_BUDGET,
        complete: true,
        out: HashMap::new(),
        read: Read::default(),
    };
    for page in pages {
        walk.resources_above(*page);
        walk.budget = PAGE_DECODE_BUDGET;
        let mut data = Vec::new();
        for id in document.get_page_contents(*page) {
            let Some(Object::Stream(stream)) = document.objects.get(&id) else {
                walk.complete = false;
                continue;
            };
            match walk.decode(stream) {
                Some(bytes) => {
                    data.extend_from_slice(&bytes);
                    data.push(b'\n');
                }
                None => walk.complete = false,
            }
        }
        let (resources, holder) =
            held_page_resources(document, *page).unwrap_or_else(|| (Dictionary::new(), *page));
        let unit = user_unit(document, *page);
        walk.content(
            &data,
            &resources,
            holder,
            Matrix([unit, 0.0, 0.0, unit, 0.0, 0.0]),
        );
    }
    if !walk.complete {
        walk.out.values_mut().for_each(|ppi| *ppi = f64::INFINITY);
    }
    Resolutions {
        ppi: walk.out,
        read: walk.read,
    }
}

/// Call `f` with every reference in `object`, a dictionary's or array's included, a stream's
/// dictionary too, but not those in objects referred to.
pub(crate) fn each_ref(object: &Object, mut f: impl FnMut(ObjectId)) {
    let mut stack = vec![object];
    while let Some(object) = stack.pop() {
        match object {
            Object::Reference(id) => f(*id),
            Object::Dictionary(dict) => stack.extend(dict.iter().map(|(_, v)| v)),
            Object::Array(values) => stack.extend(values),
            Object::Stream(stream) => stack.extend(stream.dict.iter().map(|(_, v)| v)),
            _ => {}
        }
    }
}

/// The images among `images` that may be drawn beyond what the walk read: some object the
/// document uses refers to the image, or to an object on the way to it from a page (an
/// `/XObject` dictionary, resources, a form), other than as the page and form resources the walk
/// read do. An annotation's appearance, a pattern, a Type3 glyph or a soft mask's group drawing
/// it, directly or through resources or a form it shares with a page, counts, and so does any
/// other reference. Objects nothing reaches from the trailer don't count.
pub(crate) fn used_elsewhere(
    document: &Document,
    images: &HashSet<ObjectId>,
    read: &Read,
) -> HashSet<ObjectId> {
    if images.is_empty() {
        return HashSet::new();
    }
    let targets: HashSet<ObjectId> = read.entries.iter().map(|(target, ..)| *target).collect();
    // References to each object the walk followed, by the object holding them.
    let mut held: HashMap<ObjectId, HashMap<ObjectId, usize>> = HashMap::new();
    let mut queue: Vec<ObjectId> = Vec::new();
    let mut visited = HashSet::new();
    for (_, value) in document.trailer.iter() {
        each_ref(value, |id| queue.push(id));
    }
    while let Some(id) = queue.pop() {
        if !visited.insert(id) {
            continue;
        }
        let Some(object) = document.objects.get(&id) else {
            continue;
        };
        each_ref(object, |target| {
            if targets.contains(&target) {
                *held.entry(target).or_default().entry(id).or_default() += 1;
            }
            if !visited.contains(&target) {
                queue.push(target);
            }
        });
    }
    let mut seen: HashMap<(ObjectId, ObjectId), usize> = HashMap::new();
    for (target, holder, _) in &read.entries {
        *seen.entry((*target, *holder)).or_default() += 1;
    }
    // Covered: every reference to it is one the walk followed, held by a page or node, or by an
    // object covered in turn. Everything the walk reached starts covered; whatever has a
    // reference it didn't follow, or is held by an object taken out, is taken out, until nothing
    // more is.
    let mut holds: HashMap<ObjectId, Vec<ObjectId>> = HashMap::new();
    for (target, holders) in &held {
        for holder in holders.keys() {
            holds.entry(*holder).or_default().push(*target);
        }
    }
    let mut covered: HashSet<ObjectId> = held.keys().copied().collect();
    let passes = |target: ObjectId, covered: &HashSet<ObjectId>| {
        held.get(&target).is_some_and(|holders| {
            holders.iter().all(|(holder, count)| {
                seen.get(&(target, *holder)) == Some(count)
                    && (read.roots.contains(holder) || covered.contains(holder))
            })
        })
    };
    let mut check: Vec<ObjectId> = covered.iter().copied().collect();
    while let Some(target) = check.pop() {
        if covered.contains(&target) && !passes(target, &covered) {
            covered.remove(&target);
            check.extend(holds.get(&target).into_iter().flatten().copied());
        }
    }
    images
        .iter()
        .copied()
        .filter(|image| !covered.contains(image))
        .collect()
}
