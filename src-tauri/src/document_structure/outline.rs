//! Yer imi (outline) düzenleme: ekleme, silme, yeniden adlandırma, taşıma/iç içe koyma ve hedef
//! sayfa atama.
//!
//! İstek yer imi ağacının istenen son halidir. Var olan her yer imi, düzenleyicinin PDFium'dan
//! okuduğu ağaçtaki yeriyle (indeks yolu) ve orada okunan başlıkla gelir; job worker aynı yolu
//! lopdf ile izler, başlığı karşılaştırır ve yer iminin sözlüğünü (hedef, eylem, renk, stil,
//! açık/kapalı durumu) korur. Yalnız bağlantılar (`/Parent`, `/First`, `/Last`, `/Prev`, `/Next`,
//! `/Count`), değişen başlık ve yeni hedef yazılır. Ağaçtan çıkarılan yer imleri artık
//! referanslanmaz. Sonuç PDFium'un yer imi okumasıyla doğrulanır.

use super::{check_page_count, text_of, StructureWriter};
use crate::document_io::{
    atomic_write_pdf, AtomicWriteReport, CommitState, DocumentIoError, DocumentIoFailure,
    DocumentIoStage,
};
use lopdf::{Dictionary, Object, ObjectId};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Tek istekte en fazla yer imi.
pub const MAX_OUTLINE_EDIT_NODES: usize = 10_000;
/// İstekteki en fazla iç içe yer imi düzeyi (Tauri IPC'nin JSON derinlik sınırının altında).
pub const MAX_OUTLINE_EDIT_DEPTH: usize = 32;
/// Yer imi başlığı için karakter sınırı.
pub const MAX_OUTLINE_TITLE_CHARS: usize = 1024;
/// Belgedeki yer imi ağacını okurken en fazla düğüm ve derinlik (`worker::pdfium` ile aynı).
const MAX_OUTLINE_NODES: usize = 100_000;
const MAX_OUTLINE_DEPTH: usize = 256;
/// PDFium başlıksız yer imini bu adla okur (`worker::pdfium::extract_outline`).
const UNTITLED: &str = "Untitled";

const OPERATION: &str = "set_outline";
const WHAT: &str = "yer imi düzenleme";

/// İstenen ağaçta bir yer imi.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutlineEditNode {
    pub title: String,
    /// Var olan yer imi: düzenlenen ağaçtaki yeri ve orada okunan başlık. Yeni yer iminde `None`.
    pub source: Option<OutlineSource>,
    /// Yeni hedef sayfa (0 tabanlı). `None` var olan hedefi ya da eylemi korur; yeni yer iminde
    /// zorunludur.
    pub page_index: Option<u32>,
    pub children: Vec<OutlineEditNode>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct OutlineSource {
    /// Üst düzeyden başlayan çocuk indeksleri.
    pub path: Vec<u32>,
    pub title: String,
}

/// Belgeden bağımsız istek denetimi.
pub(crate) fn validate_request(tree: &[OutlineEditNode]) -> Result<(), String> {
    let mut count = 0;
    let mut sources = HashSet::new();
    validate_level(tree, 1, &mut count, &mut sources)
}

fn validate_level<'a>(
    nodes: &'a [OutlineEditNode],
    depth: usize,
    count: &mut usize,
    sources: &mut HashSet<&'a [u32]>,
) -> Result<(), String> {
    if depth > MAX_OUTLINE_EDIT_DEPTH && !nodes.is_empty() {
        return Err(format!(
            "Yer imleri en fazla {MAX_OUTLINE_EDIT_DEPTH} düzey iç içe olabilir"
        ));
    }
    for node in nodes {
        *count += 1;
        if *count > MAX_OUTLINE_EDIT_NODES {
            return Err(format!(
                "En fazla {MAX_OUTLINE_EDIT_NODES} yer imi olabilir"
            ));
        }
        let title = node.title.trim();
        if title.is_empty() {
            return Err("Yer imi başlığı boş olamaz".to_string());
        }
        if title.chars().count() > MAX_OUTLINE_TITLE_CHARS || title.contains('\0') {
            return Err(format!(
                "Yer imi başlığı geçersiz ya da {MAX_OUTLINE_TITLE_CHARS} karakterden uzun"
            ));
        }
        match &node.source {
            Some(source) => {
                if source.path.is_empty() {
                    return Err("Yer iminin kaynak yolu boş".to_string());
                }
                if !sources.insert(source.path.as_slice()) {
                    return Err("Aynı yer imi ağaçta iki kez yer alıyor".to_string());
                }
            }
            None if node.page_index.is_none() => {
                return Err(format!("Yeni '{title}' yer iminin hedef sayfası yok"));
            }
            None => {}
        }
        validate_level(&node.children, depth + 1, count, sources)?;
    }
    Ok(())
}

/// `file_path`'teki PDF'in yer imi ağacını `tree` yapar; yerinde, atomik ve artımlı.
pub(crate) fn set_outline_report(
    file_path: String,
    tree: Vec<OutlineEditNode>,
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
    validate_request(&tree).map_err(preflight)?;
    if password.is_some() {
        return Err(preflight(format!(
            "Şifreli PDF'lerde {WHAT} desteklenmiyor"
        )));
    }
    let path = PathBuf::from(&file_path);
    let source = path.clone();
    let expected = tree.clone();
    atomic_write_pdf(
        OPERATION,
        std::slice::from_ref(&path),
        &path,
        move |pdfium, temp| write_outline(pdfium, &source, &tree, temp),
        move |document, ()| verify_saved(document, &expected),
    )
}

/// Belgedeki bir yer imi.
struct ExistingItem {
    id: ObjectId,
    title: String,
    open: bool,
}

fn write_outline(
    pdfium: &Pdfium,
    source: &Path,
    tree: &[OutlineEditNode],
    temp: &Path,
) -> Result<(), DocumentIoFailure> {
    let mut writer = StructureWriter::load(source, WHAT)?;
    let page_count = check_page_count(pdfium, source, &writer)?;
    let pages = writer.page_ids();
    let catalog = writer.catalog_id()?;
    let root = match writer.dict(catalog)?.get(b"Outlines") {
        Ok(Object::Reference(id)) => Some(*id),
        _ => None,
    };
    let existing = match root {
        Some(root) => read_existing(&writer, root)?,
        None => HashMap::new(),
    };
    if tree.is_empty() && existing.is_empty() {
        return Err(DocumentIoFailure::preflight("Belgede yer imi yok"));
    }

    // Every node gets its object id before any dictionary is written, so siblings and parents
    // can point at each other.
    let mut planned = Vec::new();
    plan(&mut writer, tree, &existing, page_count, &mut planned)?;

    let root = match root {
        Some(root) => root,
        None => {
            let root = writer.add(Object::Dictionary(Dictionary::new()));
            writer.edit(catalog, |catalog| {
                catalog.set("Outlines", Object::Reference(root));
            })?;
            root
        }
    };
    let top = link_level(&mut writer, tree, &planned, 0, root, &pages)?;
    writer.edit(root, |outlines| {
        outlines.set("Type", Object::Name(b"Outlines".to_vec()));
        set_links(outlines, &top);
    })?;
    writer.finish(temp)
}

/// Var olan yer imlerini indeks yollarıyla okur (`/First` → `/Next`, PDFium'un sırası).
fn read_existing(
    writer: &StructureWriter,
    root: ObjectId,
) -> Result<HashMap<Vec<u32>, ExistingItem>, DocumentIoFailure> {
    let mut items = HashMap::new();
    let mut seen = HashSet::from([root]);
    // (parent, its path); children are walked breadth-first so the walk keeps its own queue.
    let mut queue = vec![(root, Vec::new())];
    while let Some((parent, path)) = queue.pop() {
        if path.len() > MAX_OUTLINE_DEPTH {
            break;
        }
        let Ok(first) = writer.dict(parent).map(|dict| dict.get(b"First").cloned()) else {
            continue;
        };
        let mut next = first.ok().and_then(|first| first.as_reference().ok());
        let mut index = 0_u32;
        while let Some(id) = next {
            if !seen.insert(id) || items.len() >= MAX_OUTLINE_NODES {
                break;
            }
            let Ok(dict) = writer.dict(id) else { break };
            let title = dict
                .get(b"Title")
                .ok()
                .and_then(|title| writer.resolve(title).ok())
                .and_then(text_of)
                .unwrap_or_else(|| UNTITLED.to_string());
            let open = matches!(dict.get(b"Count"), Ok(Object::Integer(count)) if *count > 0);
            next = dict
                .get(b"Next")
                .ok()
                .and_then(|next| next.as_reference().ok());
            let mut child_path = path.clone();
            child_path.push(index);
            queue.push((id, child_path.clone()));
            items.insert(child_path, ExistingItem { id, title, open });
            index += 1;
        }
    }
    Ok(items)
}

/// Bir yer iminin yazılacak hali, ağaçta ön sırayla.
struct Planned {
    id: ObjectId,
    /// Var olan yer iminin sözlüğü (bağlantıları sonra yeniden yazılır); yenisinde boş.
    base: Dictionary,
    title_changed: bool,
    open: bool,
}

fn plan(
    writer: &mut StructureWriter,
    nodes: &[OutlineEditNode],
    existing: &HashMap<Vec<u32>, ExistingItem>,
    page_count: usize,
    planned: &mut Vec<Planned>,
) -> Result<(), DocumentIoFailure> {
    for node in nodes {
        if let Some(page) = node.page_index {
            if page as usize >= page_count {
                return Err(DocumentIoFailure::preflight(format!(
                    "'{}' yer iminin hedef sayfası {} belgede yok",
                    node.title.trim(),
                    page + 1
                )));
            }
        }
        let entry = match &node.source {
            Some(source) => {
                let item = existing.get(&source.path).ok_or_else(|| {
                    DocumentIoFailure::preflight(
                        "Yer imi belgede bulunamadı; yer imleri değişmiş olabilir",
                    )
                })?;
                if item.title != source.title {
                    return Err(DocumentIoFailure::preflight(format!(
                        "'{}' yer imi belgede '{}' olarak görünüyor; yer imleri değişmiş olabilir",
                        source.title, item.title
                    )));
                }
                let mut base = writer.dict(item.id)?.clone();
                for key in [
                    &b"Parent"[..],
                    b"First",
                    b"Last",
                    b"Prev",
                    b"Next",
                    b"Count",
                ] {
                    base.remove(key);
                }
                Planned {
                    id: item.id,
                    base,
                    title_changed: node.title.trim() != item.title,
                    open: item.open,
                }
            }
            None => Planned {
                id: writer.add(Object::Null),
                base: Dictionary::new(),
                title_changed: true,
                open: false,
            },
        };
        planned.push(entry);
        plan(writer, &node.children, existing, page_count, planned)?;
    }
    Ok(())
}

/// Bir düzeyin bağlantıları: ilk ve son çocuk ile görünür torun sayısı (§12.3.3).
struct Level {
    first: Option<ObjectId>,
    last: Option<ObjectId>,
    visible: i64,
}

fn set_links(dict: &mut Dictionary, level: &Level) {
    for key in [&b"First"[..], b"Last", b"Count"] {
        dict.remove(key);
    }
    if let (Some(first), Some(last)) = (level.first, level.last) {
        dict.set("First", Object::Reference(first));
        dict.set("Last", Object::Reference(last));
        dict.set("Count", Object::Integer(level.visible));
    }
}

/// `nodes`'u `parent`'ın çocukları olarak yazar. `planned` ön sıradadır ve `nodes[0]`'ın yeri
/// `start`'tır.
fn link_level(
    writer: &mut StructureWriter,
    nodes: &[OutlineEditNode],
    planned: &[Planned],
    start: usize,
    parent: ObjectId,
    pages: &[ObjectId],
) -> Result<Level, DocumentIoFailure> {
    // Each node's place in pre-order: after its previous sibling and all of that one's descendants.
    let mut indices = Vec::with_capacity(nodes.len());
    let mut at = start;
    for node in nodes {
        indices.push(at);
        at += 1 + count_nodes(&node.children);
    }
    let ids: Vec<ObjectId> = indices.iter().map(|&index| planned[index].id).collect();
    let mut visible = 0_i64;
    for (position, node) in nodes.iter().enumerate() {
        let entry = &planned[indices[position]];
        let children = link_level(
            writer,
            &node.children,
            planned,
            indices[position] + 1,
            entry.id,
            pages,
        )?;
        let mut dict = entry.base.clone();
        if entry.title_changed {
            dict.set("Title", lopdf::text_string(node.title.trim()));
        }
        if let Some(page) = node.page_index {
            // `/XYZ null null null`: go to the page, keeping the reader's zoom (what Acrobat
            // writes for a new bookmark). A new destination replaces the item's action.
            dict.remove(b"A");
            dict.remove(b"SE");
            dict.set(
                "Dest",
                Object::Array(vec![
                    Object::Reference(pages[page as usize]),
                    Object::Name(b"XYZ".to_vec()),
                    Object::Null,
                    Object::Null,
                    Object::Null,
                ]),
            );
        }
        dict.set("Parent", Object::Reference(parent));
        if let Some(prev) = position.checked_sub(1) {
            dict.set("Prev", Object::Reference(ids[prev]));
        }
        if let Some(next) = ids.get(position + 1) {
            dict.set("Next", Object::Reference(*next));
        }
        let open = entry.open && children.first.is_some();
        set_links(
            &mut dict,
            &Level {
                visible: if open {
                    children.visible
                } else {
                    -children.visible
                },
                ..children
            },
        );
        writer.set(entry.id, Object::Dictionary(dict));
        visible += 1 + if open { children.visible } else { 0 };
    }
    Ok(Level {
        first: ids.first().copied(),
        last: ids.last().copied(),
        visible,
    })
}

fn count_nodes(nodes: &[OutlineEditNode]) -> usize {
    nodes
        .iter()
        .map(|node| 1 + count_nodes(&node.children))
        .sum()
}

/// PDFium'un okuduğu ağaç istenenle aynı mı: yapı, başlıklar ve verilen hedef sayfalar.
fn verify_saved(
    document: &PdfDocument<'_>,
    expected: &[OutlineEditNode],
) -> Result<(), DocumentIoFailure> {
    fn level(
        mut current: Option<PdfBookmark<'_>>,
        expected: &[OutlineEditNode],
    ) -> Result<(), DocumentIoFailure> {
        for node in expected {
            let bookmark = current.take().ok_or_else(|| {
                DocumentIoFailure::invariant(format!(
                    "Kaydedilen belgede '{}' yer imi yok",
                    node.title.trim()
                ))
            })?;
            let title = bookmark.title().unwrap_or_default();
            if title != node.title.trim() {
                return Err(DocumentIoFailure::invariant(format!(
                    "Kaydedilen yer imi '{title}', beklenen '{}'",
                    node.title.trim()
                )));
            }
            if let Some(page) = node.page_index {
                let actual = bookmark
                    .destination()
                    .and_then(|destination| destination.page_index().ok());
                if actual != Some(page as u16) {
                    return Err(DocumentIoFailure::invariant(format!(
                        "'{title}' yer iminin hedefi {actual:?}, beklenen sayfa {page}"
                    )));
                }
            }
            level(bookmark.first_child(), &node.children)?;
            current = bookmark.next_sibling();
        }
        if current.is_some() {
            return Err(DocumentIoFailure::invariant(
                "Kaydedilen belgede beklenmeyen yer imi var",
            ));
        }
        Ok(())
    }
    level(document.bookmarks().root(), expected)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{pdf_with, write_temp};
    use super::*;
    use lopdf::dictionary;

    fn node(title: &str, page: Option<u32>, children: Vec<OutlineEditNode>) -> OutlineEditNode {
        OutlineEditNode {
            title: title.into(),
            source: None,
            page_index: page,
            children,
        }
    }

    fn kept(
        path: &[u32],
        title: &str,
        new_title: &str,
        children: Vec<OutlineEditNode>,
    ) -> OutlineEditNode {
        OutlineEditNode {
            title: new_title.into(),
            source: Some(OutlineSource {
                path: path.to_vec(),
                title: title.into(),
            }),
            page_index: None,
            children,
        }
    }

    fn set(path: &Path, tree: Vec<OutlineEditNode>) -> Result<AtomicWriteReport, DocumentIoError> {
        set_outline_report(path.to_string_lossy().to_string(), tree, None)
    }

    /// The outline as PDFium reads it: (depth, title, page).
    fn read(path: &Path) -> Vec<(usize, String, Option<u16>)> {
        fn walk(
            bookmark: Option<PdfBookmark<'_>>,
            depth: usize,
            out: &mut Vec<(usize, String, Option<u16>)>,
        ) {
            let mut current = bookmark;
            while let Some(bookmark) = current {
                out.push((
                    depth,
                    bookmark.title().unwrap_or_default(),
                    bookmark.destination().and_then(|d| d.page_index().ok()),
                ));
                walk(bookmark.first_child(), depth + 1, out);
                current = bookmark.next_sibling();
            }
        }
        let pdfium = crate::pdf_engine::get_pdfium().unwrap();
        let document = pdfium.load_pdf_from_file(path, None).unwrap();
        let mut out = Vec::new();
        walk(document.bookmarks().root(), 0, &mut out);
        out
    }

    /// Two top-level items, the first with a URI action and colour and one open child.
    fn sample() -> Vec<u8> {
        pdf_with(3, |doc, catalog, pages| {
            let outlines = doc.new_object_id();
            let first = doc.new_object_id();
            let child = doc.new_object_id();
            let second = doc.new_object_id();
            doc.objects.insert(first, Object::Dictionary(dictionary! {
                "Title" => lopdf::text_string("Giriş"),
                "Parent" => outlines, "Next" => second, "First" => child, "Last" => child, "Count" => 1,
                "Dest" => vec![pages[0].into(), "Fit".into()],
                "C" => vec![1.into(), 0.into(), 0.into()],
            }));
            doc.objects.insert(child, Object::Dictionary(dictionary! {
                "Title" => lopdf::text_string("Alt bölüm ş"),
                "Parent" => first,
                "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.com") },
            }));
            doc.objects.insert(
                second,
                Object::Dictionary(dictionary! {
                    "Title" => Object::string_literal("Son"),
                    "Parent" => outlines, "Prev" => first,
                    "Dest" => vec![pages[2].into(), "Fit".into()],
                }),
            );
            doc.objects.insert(
                outlines,
                Object::Dictionary(dictionary! {
                    "Type" => "Outlines", "First" => first, "Last" => second, "Count" => 3,
                }),
            );
            doc.get_object_mut(catalog)
                .unwrap()
                .as_dict_mut()
                .unwrap()
                .set("Outlines", outlines);
        })
    }

    #[test]
    fn a_new_outline_is_created_incrementally() {
        let bytes = pdf_with(3, |_, _, _| {});
        let (_dir, path) = write_temp(&bytes);
        set(
            &path,
            vec![
                node("Önsöz", Some(0), vec![node("Teşekkür", Some(1), vec![])]),
                node("Bölüm 1", Some(2), vec![]),
            ],
        )
        .unwrap();
        assert!(std::fs::read(&path).unwrap().starts_with(&bytes));
        assert_eq!(
            read(&path),
            vec![
                (0, "Önsöz".to_string(), Some(0)),
                (1, "Teşekkür".to_string(), Some(1)),
                (0, "Bölüm 1".to_string(), Some(2)),
            ]
        );
    }

    #[test]
    fn existing_items_keep_their_payload_when_moved_and_renamed() {
        let (_dir, path) = write_temp(&sample());
        // Move "Son" first, rename it, nest the old child under it, drop "Giriş".
        set(
            &path,
            vec![kept(
                &[1],
                "Son",
                "Sonuç",
                vec![kept(&[0, 0], "Alt bölüm ş", "Alt bölüm ş", vec![])],
            )],
        )
        .unwrap();
        assert_eq!(
            read(&path),
            vec![
                (0, "Sonuç".to_string(), Some(2)),
                (1, "Alt bölüm ş".to_string(), None),
            ]
        );
        let doc = lopdf::Document::load(&path).unwrap();
        let catalog = doc.catalog().unwrap();
        let root = doc
            .get_dictionary(catalog.get(b"Outlines").unwrap().as_reference().unwrap())
            .unwrap();
        // The moved item is closed (it was a leaf), so the root sees only itself.
        assert_eq!(root.get(b"Count").unwrap().as_i64().unwrap(), 1);
        let top = doc
            .get_dictionary(root.get(b"First").unwrap().as_reference().unwrap())
            .unwrap();
        let child = doc
            .get_dictionary(top.get(b"First").unwrap().as_reference().unwrap())
            .unwrap();
        // The URI action survives the move.
        assert!(child.get(b"A").unwrap().as_dict().unwrap().has(b"URI"));
        assert!(child.get(b"Next").is_err() && child.get(b"Prev").is_err());
    }

    #[test]
    fn a_new_destination_replaces_the_action_and_open_state_is_kept() {
        let (_dir, path) = write_temp(&sample());
        let mut child = kept(&[0, 0], "Alt bölüm ş", "Alt bölüm ş", vec![]);
        child.page_index = Some(1);
        set(&path, vec![kept(&[0], "Giriş", "Giriş", vec![child])]).unwrap();
        assert_eq!(
            read(&path),
            vec![
                (0, "Giriş".to_string(), Some(0)),
                (1, "Alt bölüm ş".to_string(), Some(1)),
            ]
        );
        let doc = lopdf::Document::load(&path).unwrap();
        let root = doc
            .get_dictionary(
                doc.catalog()
                    .unwrap()
                    .get(b"Outlines")
                    .unwrap()
                    .as_reference()
                    .unwrap(),
            )
            .unwrap();
        // "Giriş" was open with one child: both are visible.
        assert_eq!(root.get(b"Count").unwrap().as_i64().unwrap(), 2);
    }

    #[test]
    fn a_stale_source_is_rejected_without_touching_the_file() {
        let bytes = sample();
        let (_dir, path) = write_temp(&bytes);
        let error = set(&path, vec![kept(&[1], "Başka", "Başka", vec![])]).unwrap_err();
        assert!(
            error.detail.contains("değişmiş olabilir"),
            "{}",
            error.detail
        );
        let error = set(&path, vec![kept(&[7], "Son", "Son", vec![])]).unwrap_err();
        assert!(error.detail.contains("bulunamadı"), "{}", error.detail);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn removing_every_item_empties_the_outline() {
        let (_dir, path) = write_temp(&sample());
        set(&path, Vec::new()).unwrap();
        assert!(read(&path).is_empty());
    }

    #[test]
    fn requests_are_checked_before_the_document_is_read() {
        assert!(validate_request(&[node("  ", Some(0), vec![])]).is_err());
        assert!(validate_request(&[node("Yeni", None, vec![])]).is_err());
        let twice = kept(&[0], "A", "A", vec![]);
        assert!(validate_request(&[twice.clone(), twice]).is_err());
        let mut deep = node("x", Some(0), vec![]);
        for _ in 0..MAX_OUTLINE_EDIT_DEPTH {
            deep = node("x", Some(0), vec![deep]);
        }
        assert!(validate_request(&[deep]).is_err());
        assert!(validate_request(&[node("Tamam", Some(0), vec![])]).is_ok());
    }
}
