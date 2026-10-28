// Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors.
// Licensed under the MIT License; see src-tauri/licenses/pdfcraft.txt.
//
// Ported from PdfCraft 0.14.0, crates/organize/src/labels.rs and crates/cos/src/page_labels.rs
// (MIT OR Apache-2.0, used under MIT). Changes from the original: rewritten on lopdf (the number
// tree reader and `set_page_label_ranges`); `number_pages`, `drop_deleted_pages` and the
// `PageLabels` view are not ported; the label strings PDFium reads are the canonical labels, so
// [`label`] is used only to check a write and to preview ranges; ranges must start on page 0.

//! Page labels (ISO 32000-2 §12.4.2): "i, ii, iii, 1, 2, A-1…".
//!
//! Labels are a number tree of ranges: from page `start` on, use a numbering `style`, a `prefix`
//! and a first number. The tree is written back as one flat `/Nums` array.

use super::{check_page_count, StructureWriter};
use crate::document_io::{
    atomic_write_pdf, AtomicWriteReport, CommitState, DocumentIoError, DocumentIoFailure,
    DocumentIoStage,
};
use lopdf::{Dictionary, Document, Object, ObjectId};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A complete label, including its prefix, may contain this many UTF-8 bytes.
pub const MAX_LABEL_BYTES: usize = 1024;
/// The most ranges one request may set.
pub const MAX_LABEL_RANGES: usize = crate::job_worker::MAX_JOB_PAGES;
/// The most number-tree entries (nodes, kids and range pairs) read from one `/PageLabels` tree.
const MAX_LABEL_TREE_WORK: usize = 200_000;
/// Maximum nesting of a label number tree.
const MAX_LABEL_TREE_DEPTH: usize = 32;

const OPERATION: &str = "set_page_labels";
const WHAT: &str = "sayfa etiketi düzenleme";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PageLabelStyle {
    /// 1, 2, 3
    Decimal,
    /// I, II, III
    UpperRoman,
    /// i, ii, iii
    LowerRoman,
    /// A … Z, AA … ZZ
    UpperAlpha,
    /// a … z, aa … zz
    LowerAlpha,
    /// Prefix only (no number).
    None,
}

impl PageLabelStyle {
    fn from_name(name: &[u8]) -> Self {
        match name {
            b"D" => Self::Decimal,
            b"R" => Self::UpperRoman,
            b"r" => Self::LowerRoman,
            b"A" => Self::UpperAlpha,
            b"a" => Self::LowerAlpha,
            _ => Self::None,
        }
    }

    fn name(self) -> Option<&'static str> {
        match self {
            Self::Decimal => Some("D"),
            Self::UpperRoman => Some("R"),
            Self::LowerRoman => Some("r"),
            Self::UpperAlpha => Some("A"),
            Self::LowerAlpha => Some("a"),
            Self::None => None,
        }
    }

    /// `n` (≥ 1) in this style, or `None` when it would exceed `max_bytes`.
    fn format(self, n: u32, max_bytes: usize) -> Option<String> {
        let mut out = match self {
            Self::Decimal => n.to_string(),
            Self::UpperRoman | Self::LowerRoman => roman(u64::from(n), max_bytes)?,
            Self::UpperAlpha | Self::LowerAlpha => alpha(u64::from(n), max_bytes)?,
            Self::None => String::new(),
        };
        if out.len() > max_bytes {
            return None;
        }
        if matches!(self, Self::UpperRoman | Self::UpperAlpha) {
            out.make_ascii_uppercase();
        }
        Some(out)
    }
}

/// One labelling range.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PageLabelRange {
    /// First page (0-based) of the range.
    pub start_page: u32,
    pub style: PageLabelStyle,
    pub prefix: String,
    /// The number of the range's first page (`/St`, at least 1).
    pub first_number: u32,
}

impl PageLabelRange {
    /// The label of `page`, which must be in this range.
    fn label(&self, page: u32) -> Option<String> {
        let number = if self.style == PageLabelStyle::None {
            self.first_number
        } else {
            self.first_number
                .checked_add(page.checked_sub(self.start_page)?)?
        };
        let remaining = MAX_LABEL_BYTES.checked_sub(self.prefix.len())?;
        let mut label = self.prefix.clone();
        label.push_str(&self.style.format(number, remaining)?);
        Some(label)
    }
}

/// Lowercase Roman numerals, or `None` if the output exceeds `max_bytes`.
fn roman(mut n: u64, max_bytes: usize) -> Option<String> {
    const PARTS: [(u64, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let max_bytes = max_bytes.min(MAX_LABEL_BYTES);
    let mut out = String::new();
    for (value, part) in PARTS {
        let count = usize::try_from(n / value).ok()?;
        let bytes = count.checked_mul(part.len())?;
        if bytes > max_bytes.saturating_sub(out.len()) {
            return None;
        }
        // The quotient is checked against the tiny byte budget before repeating.
        out.push_str(&part.repeat(count));
        n %= value;
    }
    Some(out)
}

/// a..z, aa..zz, aaa.. (the repeated-letter numbering specified by PDF), or `None` if the output
/// exceeds `max_bytes`.
fn alpha(n: u64, max_bytes: usize) -> Option<String> {
    let n = n.max(1) - 1;
    let count = usize::try_from(n / 26 + 1).ok()?;
    if count > max_bytes.min(MAX_LABEL_BYTES) {
        return None;
    }
    let letter = char::from(b'a' + (n % 26) as u8);
    Some(std::iter::repeat_n(letter, count).collect())
}

/// The label `ranges` give `page` (empty when no range covers it).
pub(crate) fn label(ranges: &[PageLabelRange], page: u32) -> Option<String> {
    let index = ranges.partition_point(|range| range.start_page <= page);
    match index.checked_sub(1) {
        Some(index) => ranges[index].label(page),
        None => Some(String::new()),
    }
}

/// Belgeden bağımsız istek denetimi: aralıklar sayfa 0'dan başlar, başlangıçlar artar,
/// numaralar en az 1'dir ve her aralığın ilk etiketi yazılabilir.
pub(crate) fn validate_request(ranges: &[PageLabelRange]) -> Result<(), String> {
    if ranges.len() > MAX_LABEL_RANGES {
        return Err(format!(
            "En fazla {MAX_LABEL_RANGES} etiket aralığı verilebilir"
        ));
    }
    if ranges.first().is_some_and(|range| range.start_page != 0) {
        return Err("İlk etiket aralığı 1. sayfadan başlamalı".to_string());
    }
    if ranges
        .windows(2)
        .any(|pair| pair[0].start_page >= pair[1].start_page)
    {
        return Err("Etiket aralıklarının başlangıç sayfaları artan ve tekil olmalı".to_string());
    }
    for range in ranges {
        if range.first_number == 0 {
            return Err("Etiket numarası en az 1 olmalı".to_string());
        }
        if range.prefix.contains('\0') {
            return Err("Etiket öneki NUL karakteri içeremez".to_string());
        }
        if range.label(range.start_page).is_none() {
            return Err(format!(
                "Sayfa {} etiketi {MAX_LABEL_BYTES} bayt sınırını aşıyor",
                range.start_page + 1
            ));
        }
    }
    Ok(())
}

/// `file_path`'teki PDF'in sayfa etiketlerini `ranges` ile değiştirir (boş liste
/// `/PageLabels`'ı kaldırır); yerinde, atomik ve artımlı.
pub(crate) fn set_page_labels_report(
    file_path: String,
    ranges: Vec<PageLabelRange>,
    password: Option<String>,
) -> Result<AtomicWriteReport, DocumentIoError> {
    let preflight = |detail: String| {
        DocumentIoError::new(
            OPERATION,
            DocumentIoStage::Preflight,
            CommitState::Prepared,
            detail,
        )
    };
    validate_request(&ranges).map_err(preflight)?;
    if password.is_some() {
        return Err(preflight(format!(
            "Şifreli PDF'lerde {WHAT} desteklenmiyor"
        )));
    }
    let path = PathBuf::from(&file_path);
    let source = path.clone();
    let expected = ranges.clone();
    atomic_write_pdf(
        OPERATION,
        std::slice::from_ref(&path),
        &path,
        move |pdfium, temp| write_labels(pdfium, &source, &ranges, temp),
        move |document, &page_count| verify_saved(document, &expected, page_count),
    )
}

fn write_labels(
    pdfium: &Pdfium,
    source: &Path,
    ranges: &[PageLabelRange],
    temp: &Path,
) -> Result<u32, DocumentIoFailure> {
    let mut writer = StructureWriter::load(source, WHAT)?;
    let page_count = check_page_count(pdfium, source, &writer)?;
    if let Some(range) = ranges
        .iter()
        .find(|range| range.start_page as usize >= page_count)
    {
        return Err(DocumentIoFailure::preflight(format!(
            "Etiket aralığı sayfa {} belgede yok (sayfa sayısı {page_count})",
            range.start_page + 1
        )));
    }
    let catalog = writer.catalog_id()?;
    if ranges.is_empty() {
        if !writer.dict(catalog)?.has(b"PageLabels") {
            return Err(DocumentIoFailure::preflight("Belgede sayfa etiketi yok"));
        }
        writer.edit(catalog, |catalog| {
            catalog.remove(b"PageLabels");
        })?;
    } else {
        let mut nums = Vec::with_capacity(ranges.len() * 2);
        for range in ranges {
            let mut spec = Dictionary::new();
            if let Some(style) = range.style.name() {
                spec.set("S", Object::Name(style.as_bytes().to_vec()));
            }
            if !range.prefix.is_empty() {
                spec.set("P", lopdf::text_string(&range.prefix));
            }
            if range.first_number != 1 {
                spec.set("St", Object::Integer(i64::from(range.first_number)));
            }
            nums.push(Object::Integer(i64::from(range.start_page)));
            // Indirect, as Acrobat writes them: a direct dictionary is valid too, but some
            // readers skip it and show physical page numbers.
            nums.push(Object::Reference(writer.add(Object::Dictionary(spec))));
        }
        let mut tree = Dictionary::new();
        tree.set("Nums", Object::Array(nums));
        let tree = writer.add(Object::Dictionary(tree));
        writer.edit(catalog, |catalog| {
            catalog.set("PageLabels", Object::Reference(tree));
        })?;
    }
    writer.finish(temp)?;
    Ok(page_count as u32)
}

/// PDFium'un okuduğu etiketler beklenenle aynı mı: her aralığın ilk iki sayfası ve son sayfa.
fn verify_saved(
    document: &PdfDocument<'_>,
    ranges: &[PageLabelRange],
    page_count: u32,
) -> Result<(), DocumentIoFailure> {
    let mut pages: Vec<u32> = ranges
        .iter()
        .flat_map(|range| [range.start_page, range.start_page + 1])
        .chain([0, page_count.saturating_sub(1)])
        .filter(|&page| page < page_count)
        .collect();
    pages.sort_unstable();
    pages.dedup();
    for page_index in pages {
        let page = document
            .pages()
            .get(page_index as u16)
            .map_err(|error| DocumentIoFailure::invariant(format!("Sayfa okunamadı: {error}")))?;
        let actual = page.label().unwrap_or_default();
        let expected = label(ranges, page_index).unwrap_or_default();
        if actual != expected {
            return Err(DocumentIoFailure::invariant(format!(
                "Sayfa {} etiketi '{actual}', beklenen '{expected}'",
                page_index + 1
            )));
        }
    }
    Ok(())
}

/// Session snapshot'ının etiket aralıkları (salt okunur job). Şifreli belgede parola yükleme
/// sırasında verilir; snapshot değiştirilmez.
pub(crate) fn inspect_snapshot(
    snapshot_path: &Path,
    credential: Option<&str>,
) -> Result<Vec<PageLabelRange>, String> {
    let bytes = std::fs::read(snapshot_path).map_err(|error| format!("PDF okunamadı: {error}"))?;
    if bytes.len() > super::MAX_STRUCTURE_INPUT_BYTES {
        return Err("PDF sayfa etiketlerini okumak için çok büyük (256 MiB sınırı)".to_string());
    }
    let document = crate::lopdf_compat::load_mem_with_credential(&bytes, credential)
        .map_err(|error| format!("PDF yapısı ayrıştırılamadı: {error:?}"))?;
    read_ranges(&document)
}

/// The document's label ranges, sorted by start page (empty when it has no `/PageLabels`).
/// A start page given twice keeps its later range.
pub(crate) fn read_ranges(document: &Document) -> Result<Vec<PageLabelRange>, String> {
    let catalog = document
        .catalog()
        .map_err(|_| "Belge kataloğu bulunamadı".to_string())?;
    let Ok(tree) = catalog.get(b"PageLabels") else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    let mut reader = TreeReader {
        document,
        work_left: MAX_LABEL_TREE_WORK,
        seen: HashSet::new(),
    };
    reader.node(tree, 0, &mut entries)?;
    entries.sort_by_key(|range: &PageLabelRange| range.start_page);
    let mut unique: Vec<PageLabelRange> = Vec::with_capacity(entries.len());
    for range in entries {
        match unique.last_mut() {
            Some(last) if last.start_page == range.start_page => *last = range,
            _ => unique.push(range),
        }
    }
    Ok(unique)
}

struct TreeReader<'a> {
    document: &'a Document,
    work_left: usize,
    seen: HashSet<ObjectId>,
}

impl<'a> TreeReader<'a> {
    fn work(&mut self, count: usize) -> Result<(), String> {
        self.work_left = self
            .work_left
            .checked_sub(count)
            .ok_or_else(|| "Sayfa etiketi ağacında çok fazla girdi var".to_string())?;
        Ok(())
    }

    /// `object`, referanslar (sınırlı) izlenerek.
    fn resolve(&self, mut object: &'a Object) -> Result<&'a Object, String> {
        for _ in 0..MAX_LABEL_TREE_DEPTH {
            match object {
                Object::Reference(id) => {
                    object = self
                        .document
                        .get_object(*id)
                        .map_err(|_| "Sayfa etiketi nesnesi bulunamadı".to_string())?;
                }
                other => return Ok(other),
            }
        }
        Err("Sayfa etiketi referansı çok derin".to_string())
    }

    fn node(
        &mut self,
        node: &'a Object,
        depth: usize,
        entries: &mut Vec<PageLabelRange>,
    ) -> Result<(), String> {
        self.work(1)?;
        if depth > MAX_LABEL_TREE_DEPTH {
            return Err("Sayfa etiketi ağacı çok derin".to_string());
        }
        if let Object::Reference(id) = node {
            if !self.seen.insert(*id) {
                return Err("Sayfa etiketi ağacı bir düğümü tekrarlıyor".to_string());
            }
        }
        let Ok(dict) = self.resolve(node)?.as_dict() else {
            return Ok(());
        };
        if let Ok(nums) = dict.get(b"Nums") {
            if let Ok(nums) = self.resolve(nums)?.as_array() {
                self.work(nums.len().div_ceil(2))?;
                for [start, spec] in nums.as_chunks::<2>().0 {
                    let Ok(start) = self.resolve(start)?.as_i64() else {
                        continue;
                    };
                    let Ok(start_page) = u32::try_from(start) else {
                        continue;
                    };
                    let Ok(spec) = self.resolve(spec)?.as_dict() else {
                        continue;
                    };
                    entries.push(self.range(start_page, spec)?);
                }
            }
        }
        if let Ok(kids) = dict.get(b"Kids") {
            if let Ok(kids) = self.resolve(kids)?.as_array() {
                self.work(kids.len())?;
                for kid in kids {
                    self.node(kid, depth + 1, entries)?;
                }
            }
        }
        Ok(())
    }

    fn range(&self, start_page: u32, spec: &Dictionary) -> Result<PageLabelRange, String> {
        let prefix = match spec.get(b"P") {
            Ok(prefix) => super::text_of(self.resolve(prefix)?).unwrap_or_default(),
            Err(_) => String::new(),
        };
        if prefix.len() > MAX_LABEL_BYTES {
            return Err(format!(
                "Etiket öneki {MAX_LABEL_BYTES} bayt sınırını aşıyor"
            ));
        }
        let style = match spec.get(b"S").map(|style| self.resolve(style)) {
            Ok(Ok(Object::Name(name))) => PageLabelStyle::from_name(name),
            _ => PageLabelStyle::None,
        };
        let first_number = match spec.get(b"St").map(|first| self.resolve(first)) {
            Ok(Ok(Object::Integer(first))) => (*first).clamp(1, i64::from(u32::MAX)) as u32,
            _ => 1,
        };
        Ok(PageLabelRange {
            start_page,
            style,
            prefix,
            first_number,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{pdf_with, write_temp};
    use super::*;
    use lopdf::dictionary;

    fn range(
        start_page: u32,
        style: PageLabelStyle,
        prefix: &str,
        first_number: u32,
    ) -> PageLabelRange {
        PageLabelRange {
            start_page,
            style,
            prefix: prefix.into(),
            first_number,
        }
    }

    fn set(path: &Path, ranges: Vec<PageLabelRange>) -> Result<AtomicWriteReport, DocumentIoError> {
        set_page_labels_report(path.to_string_lossy().to_string(), ranges, None)
    }

    #[test]
    fn numerals_follow_pdf_numbering() {
        assert_eq!(roman(4, MAX_LABEL_BYTES).as_deref(), Some("iv"));
        assert_eq!(roman(1994, MAX_LABEL_BYTES).as_deref(), Some("mcmxciv"));
        for (n, expected) in [
            (0, "a"),
            (1, "a"),
            (26, "z"),
            (27, "aa"),
            (52, "zz"),
            (53, "aaa"),
        ] {
            assert_eq!(alpha(n, MAX_LABEL_BYTES).as_deref(), Some(expected));
        }
        assert_eq!(
            PageLabelStyle::UpperRoman.format(14, 64).as_deref(),
            Some("XIV")
        );
        assert_eq!(
            PageLabelStyle::UpperAlpha.format(28, 64).as_deref(),
            Some("BB")
        );
        // A numeral that doesn't fit the budget is refused rather than cut.
        assert_eq!(roman(u64::from(u32::MAX), MAX_LABEL_BYTES), None);
    }

    #[test]
    fn labels_come_from_the_covering_range() {
        let ranges = vec![
            range(0, PageLabelStyle::LowerRoman, "", 1),
            range(3, PageLabelStyle::Decimal, "Ek-", 5),
            range(6, PageLabelStyle::None, "Kapak", 1),
        ];
        let labels: Vec<_> = (0..7).map(|page| label(&ranges, page).unwrap()).collect();
        assert_eq!(labels, ["i", "ii", "iii", "Ek-5", "Ek-6", "Ek-7", "Kapak"]);
        assert_eq!(label(&[], 4).as_deref(), Some(""));
    }

    #[test]
    fn requests_are_checked_before_the_document_is_read() {
        assert!(validate_request(&[]).is_ok());
        assert!(validate_request(&[range(1, PageLabelStyle::Decimal, "", 1)]).is_err());
        assert!(validate_request(&[
            range(0, PageLabelStyle::Decimal, "", 1),
            range(0, PageLabelStyle::Decimal, "", 1),
        ])
        .is_err());
        assert!(validate_request(&[range(0, PageLabelStyle::Decimal, "", 0)]).is_err());
        assert!(validate_request(&[range(0, PageLabelStyle::None, &"x".repeat(1025), 1)]).is_err());
    }

    #[test]
    fn ranges_are_written_read_back_and_seen_by_pdfium() {
        let bytes = pdf_with(5, |_, _, _| {});
        let (_dir, path) = write_temp(&bytes);
        let ranges = vec![
            range(0, PageLabelStyle::LowerRoman, "", 1),
            range(2, PageLabelStyle::Decimal, "Bölüm ", 1),
        ];
        set(&path, ranges.clone()).unwrap();
        let output = std::fs::read(&path).unwrap();
        assert!(
            output.starts_with(&bytes),
            "original bytes must stay in place"
        );
        let document = Document::load_mem(&output).unwrap();
        assert_eq!(read_ranges(&document).unwrap(), ranges);

        // An empty list removes the labels again.
        set(&path, Vec::new()).unwrap();
        let document = Document::load(&path).unwrap();
        assert_eq!(read_ranges(&document).unwrap(), Vec::new());
    }

    #[test]
    fn a_range_past_the_last_page_is_rejected_without_touching_the_file() {
        let bytes = pdf_with(2, |_, _, _| {});
        let (_dir, path) = write_temp(&bytes);
        let error = set(
            &path,
            vec![
                range(0, PageLabelStyle::Decimal, "", 1),
                range(2, PageLabelStyle::Decimal, "", 1),
            ],
        )
        .unwrap_err();
        assert!(error.detail.contains("belgede yok"), "{}", error.detail);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn nested_and_hostile_trees_are_read_within_limits() {
        let bytes = pdf_with(4, |doc, catalog, _| {
            let roman = doc.add_object(dictionary! { "S" => "r" });
            let leaf = doc.add_object(dictionary! {
                "Nums" => vec![0.into(), roman.into(), 2.into(), dictionary! { "S" => "D", "St" => 3, "P" => Object::string_literal("A-") }.into()],
            });
            let tree = doc.add_object(dictionary! { "Kids" => vec![leaf.into()] });
            doc.get_object_mut(catalog)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("PageLabels", tree);
        });
        let document = Document::load_mem(&bytes).unwrap();
        assert_eq!(
            read_ranges(&document).unwrap(),
            vec![
                range(0, PageLabelStyle::LowerRoman, "", 1),
                range(2, PageLabelStyle::Decimal, "A-", 3),
            ]
        );

        // A tree whose kid is the tree itself is refused, not walked forever.
        let cyclic = pdf_with(1, |doc, catalog, _| {
            let tree = doc.new_object_id();
            doc.objects.insert(
                tree,
                Object::Dictionary(dictionary! { "Kids" => vec![Object::Reference(tree)] }),
            );
            doc.get_object_mut(catalog)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("PageLabels", tree);
        });
        let document = Document::load_mem(&cyclic).unwrap();
        assert!(read_ranges(&document).is_err());
    }
}
