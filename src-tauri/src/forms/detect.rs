// Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors.
// Licensed under the MIT License; see src-tauri/licenses/pdfcraft.txt.
//
// Ported from PdfCraft 0.16.1, crates/forms/src/detect.rs (MIT OR Apache-2.0, used under MIT).
// Changes from the original: the shapes and words are read with PDFium page objects and PDFium
// text instead of PdfCraft's content-stream parser and renderer (`page_shapes`, `page_words`);
// rewritten for Rust edition 2021 (no let chains); `detect` takes the taken names as a set;
// boxes are also built from separate horizontal and vertical lines (word-processor tables) and
// their edges stop being rules; comb cells may differ in width by up to 1 pt (was 0.5); a
// blank's field stops at the bottom of a box right above it;
// point comparisons use a 1e-3 pt tolerance because PDFium hands back f32 coordinates; the
// number of objects read and shapes kept is bounded; the raster (scanned page) path is not
// ported.

//! Form tasarımı ▸ otomatik alan algılama: basılı bir formun yanıt beklediği yerleri bulur
//! (alt çizgi dizileri, yatay çizgiler, boş kutular, tek karakterlik hücre sıraları ve küçük
//! kareler) ve her yeni alanı yanındaki etikete göre adlandırır (Acrobat'ın otomatik
//! adlandırması).
//!
//! [`detect`] sayfanın kelimeleri ve çizilmiş şekilleri üzerinde saf bir fonksiyondur. Girdiyi
//! [`read_page`] PDFium ile okur: kelimeler PDFium metninden, şekiller PDFium path nesnelerinden
//! (form XObject'lerinin içi dahil). Yazım `forms::author`'ın işidir.

use pdfium_render::prelude::*;
use std::collections::HashSet;

/// Sayfadaki bir metin parçası, PDF user space'te.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    /// [x0, y0, x1, y1], y yukarı.
    pub rect: [f64; 4],
}

/// Sayfada çizilmiş şekiller, user space'te: eksene hizalı dikdörtgen yollar ve yatay
/// doğru parçaları.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shapes {
    pub boxes: Vec<[f64; 4]>,
    pub rules: Vec<[f64; 4]>,
}

/// Algılamanın önerdiği alan türü.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    CheckBox,
    /// Birbirine değen, her biri tek karakterlik hücre sırası: comb bayraklı ve karakter sınırı
    /// `cells` olan bir metin alanı.
    Comb {
        cells: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub kind: Kind,
    pub rect: [f64; 4],
    /// Yakındaki etiketten, geçerli bir alan adına çevrilmiş.
    pub name: String,
}

/// Algılamanın bir sayfadan okuduğu her şey.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageInput {
    pub words: Vec<Word>,
    pub shapes: Shapes,
    /// Sayfadaki mevcut widget'ların dikdörtgenleri; aday bunlarla örtüşemez.
    pub existing: Vec<[f64; 4]>,
}

/// Bir alt yolun dikdörtgen sayılabilmesi için en fazla nokta sayısı: dört köşe ve kenarları
/// üzerindeki noktalar (#2942: her dikey kenarının ortasında bir nokta olan hücre).
const MAX_RECT_POINTS: usize = 16;
/// İki noktanın aynı sayıldığı uzaklık (pt). PDFium koordinatları f32'dir.
const NEAR: f64 = 1e-3;
/// Bir sayfada okunan en fazla sayfa nesnesi (form XObject'lerinin içindekiler dahil).
const MAX_OBJECTS: usize = 200_000;
/// İç içe form XObject derinliği.
const MAX_FORM_DEPTH: usize = 16;
/// Tutulan en fazla kutu ve çizgi sayısı (her biri için).
const MAX_SHAPES: usize = 4_000;
/// Okunan en fazla kelime.
const MAX_WORDS: usize = 100_000;
/// Ayrı çizgilerden hücre kurarken iki çizginin buluştuğu sayıldığı uzaklık (pt): kelime
/// işlemcilerin tablo kenarlıkları köşelerde kenar kalınlığı kadar taşar ya da eksik kalır.
const JOINT: f64 = 1.5;
/// Hücre kurmak için okunan en fazla yatay ve dikey çizgi (her biri için).
const MAX_LINES: usize = 20_000;
/// Hücre kurmanın en fazla adımı; çok büyük ızgaralarda iş burada kesilir.
const MAX_CELL_STEPS: usize = 4_000_000;

/// Okuma sırasında toplananlar. Kelime işlemciler (Word, LibreOffice) tablo kenarlıklarını
/// dikdörtgen olarak değil, ayrı çizgiler olarak çizer; hücreler bu çizgilerden kurulur.
#[derive(Default)]
struct Reading {
    shapes: Shapes,
    /// Dikey çizgiler `[x, y0, x, y1]`.
    columns: Vec<[f64; 4]>,
    /// Dikdörtgenlerin yatay kenarları `[x0, y, x1, y]` (kural sayılmazlar).
    edges: Vec<[f64; 4]>,
}

impl Reading {
    fn column(&mut self, x: f64, y0: f64, y1: f64) {
        if self.columns.len() < MAX_LINES {
            self.columns.push([x, y0.min(y1), x, y0.max(y1)]);
        }
    }

    /// Dikdörtgenin kenarları da çizgidir: içinden dikey çizgilerle bölünen satır kutuları.
    fn rectangle_edges(&mut self, r: [f64; 4]) {
        if self.edges.len() + 2 <= MAX_LINES {
            self.edges.push([r[0], r[1], r[2], r[1]]);
            self.edges.push([r[0], r[3], r[2], r[3]]);
        }
        self.column(r[0], r[1], r[3]);
        self.column(r[2], r[1], r[3]);
    }
}

/// Algılama için bir alt yol.
#[derive(Default)]
struct DetectionPath {
    points: Vec<[f64; 2]>,
    rules: Vec<[f64; 4]>,
    /// Dikey doğru parçaları `[x, y0, x, y1]`.
    columns: Vec<[f64; 4]>,
    complex: bool,
    closed: bool,
}

impl DetectionPath {
    fn finish(&mut self, paths: &mut Vec<Self>) {
        let path = std::mem::take(self);
        if !path.rules.is_empty()
            || !path.columns.is_empty()
            || (!path.complex && path.points.len() >= 4)
        {
            paths.push(path);
        }
    }

    fn rectangle(&self, implicitly_closed: bool) -> Option<[f64; 4]> {
        if self.complex {
            return None;
        }
        if self.points.iter().flatten().any(|v| !v.is_finite()) {
            return None;
        }
        let near =
            |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < NEAR && (a[1] - b[1]).abs() < NEAR;
        let mut points = self.points.clone();
        let repeated_end = points.len() > 1 && near(points[0], *points.last()?);
        if repeated_end {
            points.pop();
        }
        if !(self.closed || implicitly_closed || repeated_end) {
            return None;
        }
        // Düz bir kenarın üzerinde, komşularının arasındaki nokta şekli değiştirmez: atılır;
        // böylece kenarının ortasında noktası olan hücre göründüğü dikdörtgen olur (#2942).
        // Kenar boyunca geri dönen nokta komşularının arasında değildir ve kalır; geri dönen
        // yol yine dikdörtgen sayılmaz.
        let between = |a: f64, m: f64, b: f64| a.min(b) - NEAR <= m && m <= a.max(b) + NEAR;
        let mut i = 0;
        while points.len() > 4 && i < points.len() {
            let n = points.len();
            let (a, m, b) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n]);
            let on_row = (a[1] - m[1]).abs() < NEAR
                && (m[1] - b[1]).abs() < NEAR
                && between(a[0], m[0], b[0]);
            let on_column = (a[0] - m[0]).abs() < NEAR
                && (m[0] - b[0]).abs() < NEAR
                && between(a[1], m[1], b[1]);
            if on_row || on_column {
                points.remove(i);
                i = 0;
            } else {
                i += 1;
            }
        }
        if points.len() != 4 {
            return None;
        }
        let mut horizontal = [false; 4];
        for i in 0..4 {
            let (a, b) = (points[i], points[(i + 1) % 4]);
            let h = (a[1] - b[1]).abs() < NEAR;
            let v = (a[0] - b[0]).abs() < NEAR;
            if h == v {
                return None; // Çapraz ya da sıfır uzunlukta kenar.
            }
            horizontal[i] = h;
        }
        if (0..4).any(|i| horizontal[i] == horizontal[(i + 1) % 4]) {
            return None;
        }
        Some([
            points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min),
            points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min),
            points
                .iter()
                .map(|p| p[0])
                .fold(f64::NEG_INFINITY, f64::max),
            points
                .iter()
                .map(|p| p[1])
                .fold(f64::NEG_INFINITY, f64::max),
        ])
    }
}

// ---------------------------------------------------------------------------
// PDFium girdisi
// ---------------------------------------------------------------------------

/// `[a, b, c, d, e, f]`: x' = a·x + c·y + e, y' = b·x + d·y + f.
type Matrix = [f64; 6];

fn matrix(value: PdfMatrix) -> Matrix {
    [
        value.a(),
        value.b(),
        value.c(),
        value.d(),
        value.e(),
        value.f(),
    ]
    .map(f64::from)
}

/// Önce `inner`, sonra `outer`.
fn then(inner: Matrix, outer: Matrix) -> Matrix {
    let [a, b, c, d, e, f] = inner;
    let [oa, ob, oc, od, oe, of] = outer;
    [
        a * oa + b * oc,
        a * ob + b * od,
        c * oa + d * oc,
        c * ob + d * od,
        e * oa + f * oc + oe,
        e * ob + f * od + of,
    ]
}

fn apply(m: Matrix, [x, y]: [f64; 2]) -> [f64; 2] {
    [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]
}

/// Sayfanın algılama girdisi: kelimeler, şekiller ve mevcut widget'lar.
pub(crate) fn read_page(page: &PdfPage<'_>) -> Result<PageInput, String> {
    Ok(PageInput {
        words: page_words(page)?,
        shapes: page_shapes(page),
        existing: page
            .annotations()
            .iter()
            .filter(|annotation| annotation.annotation_type() == PdfPageAnnotationType::Widget)
            .filter_map(|annotation| annotation.bounds().ok())
            .map(|bounds| {
                [bounds.left(), bounds.bottom(), bounds.right(), bounds.top()]
                    .map(|value| f64::from(value.value))
            })
            .collect(),
    })
}

/// Sayfanın kelimeleri, user space'te; alt çizgi dizileri çevrelerindeki metinden ayrı
/// kelimelerdir (`Ad:____` → `Ad:`, `____`).
fn page_words(page: &PdfPage<'_>) -> Result<Vec<Word>, String> {
    let text = page
        .text()
        .map_err(|error| format!("Sayfa metni okunamadı: {error}"))?;
    let mut words = Vec::new();
    crate::worker::pdfium::for_each_word_breaking(
        &text,
        |previous, next| (previous == '_') != (next == '_'),
        |word, [x0, y0, x1, y1]| {
            if !word.is_empty() && x0 < x1 && y0 < y1 && words.len() < MAX_WORDS {
                words.push(Word {
                    text: word.to_string(),
                    rect: [x0, y0, x1, y1].map(f64::from),
                });
            }
            Ok::<_, std::convert::Infallible>(())
        },
    )
    .unwrap_or_else(|never| match never {});
    Ok(words)
}

/// Sayfanın çizdiği (kontur ya da dolgu) dikdörtgenler ve yatay doğru parçaları, user space'te;
/// ayrı çizgilerle çizilmiş tablo hücreleri de kutudur ve kenarları kural sayılmaz.
fn page_shapes(page: &PdfPage<'_>) -> Shapes {
    let mut out = Reading::default();
    let mut budget = MAX_OBJECTS;
    let objects = page.objects();
    for index in 0..objects.len() {
        if budget == 0 {
            break;
        }
        if let Ok(object) = objects.get(index) {
            read_object(
                &object,
                [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                0,
                &mut budget,
                &mut out,
            );
        }
    }
    let Reading {
        mut shapes,
        columns,
        mut edges,
    } = out;
    shapes.rules.truncate(MAX_LINES);
    edges.extend(shapes.rules.iter().copied());
    let cells: Vec<[f64; 4]> = cells_from_lines(&edges, &columns)
        .into_iter()
        .filter(|cell| !shapes.boxes.iter().any(|b| same_box(*b, *cell)))
        .collect();
    shapes.rules = trim_edges(&shapes.rules, &cells);
    shapes.boxes.extend(cells);
    shapes.boxes.truncate(MAX_SHAPES);
    shapes.rules.truncate(MAX_SHAPES);
    shapes
}

fn read_object(
    object: &PdfPageObject<'_>,
    outer: Matrix,
    depth: usize,
    budget: &mut usize,
    out: &mut Reading,
) {
    *budget = budget.saturating_sub(1);
    if let Some(path) = object.as_path_object() {
        if let Ok(own) = path.matrix() {
            read_path(path, then(matrix(own), outer), out);
        }
    } else if let Some(form) = object.as_x_object_form_object() {
        if depth >= MAX_FORM_DEPTH {
            return;
        }
        let Ok(own) = form.matrix() else { return };
        let ctm = then(matrix(own), outer);
        for index in 0..form.len() {
            if *budget == 0 {
                return;
            }
            if let Ok(child) = form.get(index) {
                read_object(&child, ctm, depth + 1, budget, out);
            }
        }
    }
}

/// Bir path nesnesinin alt yolları; PdfCraft'ın içerik akışı okuyucusunun (`m`, `l`, `c`,
/// `h` ve boyama operatörleri) PDFium segmentleri üzerindeki karşılığı. PDFium `re`'yi dört
/// kenarlı kapalı bir alt yol olarak verir.
fn read_path(path: &PdfPagePathObject<'_>, ctm: Matrix, out: &mut Reading) {
    let filled = !matches!(path.fill_mode(), Ok(PdfPathFillMode::None) | Err(_));
    let stroked = path.is_stroked().unwrap_or(false);
    if !filled && !stroked {
        return;
    }
    let mut subpaths: Vec<DetectionPath> = Vec::new();
    let mut current = DetectionPath::default();
    let mut cursor: Option<[f64; 2]> = None;
    let segments = path.segments();
    for index in 0..segments.len() {
        let Ok(segment) = segments.get(index) else {
            current.complex = true;
            continue;
        };
        let point = apply(
            ctm,
            [f64::from(segment.x().value), f64::from(segment.y().value)],
        );
        match segment.segment_type() {
            PdfPathSegmentType::MoveTo => {
                current.finish(&mut subpaths);
                current.points.push(point);
                cursor = Some(point);
            }
            PdfPathSegmentType::LineTo => {
                // Kapattıktan sonra devam eden yol tek bir dört kenarlı dikdörtgen değildir.
                if current.closed {
                    current.complex = true;
                }
                if let Some(start) = cursor {
                    if !current.complex {
                        if current.points.len() > MAX_RECT_POINTS {
                            current.complex = true;
                        } else {
                            if current.points.is_empty() {
                                current.points.push(start);
                            }
                            current.points.push(point);
                        }
                    }
                    if (start[1] - point[1]).abs() < 0.5 && (start[0] - point[0]).abs() > 1.0 {
                        current.rules.push([
                            start[0].min(point[0]),
                            start[1],
                            start[0].max(point[0]),
                            start[1],
                        ]);
                    } else if (start[0] - point[0]).abs() < 0.5 && (start[1] - point[1]).abs() > 1.0
                    {
                        current.columns.push([
                            start[0],
                            start[1].min(point[1]),
                            start[0],
                            start[1].max(point[1]),
                        ]);
                    }
                }
                cursor = Some(point);
            }
            PdfPathSegmentType::BezierTo | PdfPathSegmentType::Unknown => {
                current.complex = true;
                cursor = Some(point);
            }
        }
        if segment.is_close() {
            current.closed = true;
            cursor = current.points.first().copied().or(cursor);
        }
    }
    current.finish(&mut subpaths);

    let mut rules = Vec::new();
    for subpath in subpaths {
        match subpath.rectangle(filled) {
            // Çok yassı dikdörtgen bir çizgidir.
            Some(r) if r[3] - r[1] < 1.5 && r[2] - r[0] > 1.0 => {
                let y = (r[1] + r[3]) / 2.0;
                out.shapes.rules.push([r[0], y, r[2], y]);
            }
            // Çok dar dikdörtgen dikey bir çizgidir (kelime işlemcilerin tablo kenarlıkları).
            Some(r) if r[2] - r[0] < 1.5 && r[3] - r[1] > 1.0 => {
                out.column((r[0] + r[2]) / 2.0, r[1], r[3]);
            }
            Some(r) => {
                out.shapes.boxes.push(r);
                out.rectangle_edges(r);
            }
            None => {
                rules.extend(subpath.rules);
                for c in subpath.columns {
                    out.column(c[0], c[1], c[3]);
                }
            }
        }
    }
    out.shapes.rules.append(&mut rules);
}

/// İki kutu aynı yerde mi (köşeleri [`JOINT`] içinde).
fn same_box(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= JOINT)
}

/// Aynı doğru üzerinde birbirine değen ya da örtüşen parçalar tek çizgidir. `lines` yatayken
/// `[x0, y, x1, y]`, dikeyken `[x, y0, x, y1]` (`vertical`); sonuç `(konum, baş, son)`
/// üçlüleri, konuma göre sıralı.
fn merge_lines(lines: &[[f64; 4]], vertical: bool) -> Vec<[f64; 3]> {
    let mut parts: Vec<[f64; 3]> = lines
        .iter()
        .filter(|l| l.iter().all(|v| v.is_finite()))
        .map(|l| {
            if vertical {
                [l[0], l[1].min(l[3]), l[1].max(l[3])]
            } else {
                [l[1], l[0].min(l[2]), l[0].max(l[2])]
            }
        })
        .collect();
    parts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    let mut merged: Vec<[f64; 3]> = Vec::with_capacity(parts.len());
    // Aynı konumdaki (yarım nokta içinde) çizgiler başlangıçlarına göre sıralı gelsin diye,
    // konumu yakın olan grup ayrıca başa göre sıralanır.
    let mut start = 0;
    while start < parts.len() {
        let mut end = start + 1;
        while end < parts.len() && parts[end][0] - parts[start][0] <= 0.5 {
            end += 1;
        }
        let group = &mut parts[start..end];
        group.sort_by(|a, b| a[1].total_cmp(&b[1]));
        let first = merged.len();
        for part in group.iter() {
            match merged[first..].last_mut() {
                Some(last) if part[1] <= last[2] + JOINT => last[2] = last[2].max(part[2]),
                _ => merged.push(*part),
            }
        }
        start = end;
    }
    merged.sort_by(|a, b| a[0].total_cmp(&b[0]));
    merged
}

/// Ayrı yatay (`horizontal`, `[x0, y, x1, y]`) ve dikey (`vertical`, `[x, y0, x, y1]`)
/// çizgilerin kapattığı en küçük hücreler. Her hücre sol üst köşesinden bulunur: bir dikey
/// çizginin değdiği her yatay çizgi bir üst kenardır; alt kenar o dikey çizginin ulaştığı en
/// yakın alttaki yatay çizgi, sağ kenar ise iki kenarın da ulaştığı ve aradaki yüksekliğin
/// tamamını kapatan en yakın dikey çizgidir.
fn cells_from_lines(horizontal: &[[f64; 4]], vertical: &[[f64; 4]]) -> Vec<[f64; 4]> {
    let rows = merge_lines(horizontal, false);
    let columns = merge_lines(vertical, true);
    let covers = |line: &[f64; 3], at: f64| line[1] - JOINT <= at && at <= line[2] + JOINT;
    let mut steps = 0usize;
    let mut cells: Vec<[f64; 4]> = Vec::new();
    'columns: for left in &columns {
        let x = left[0];
        // Üst kenarlar yukarıdan aşağı.
        for (top_index, top) in rows.iter().enumerate().rev() {
            steps += 1;
            if steps > MAX_CELL_STEPS || cells.len() >= MAX_SHAPES {
                break 'columns;
            }
            if top[0] > left[2] + JOINT {
                continue;
            }
            if top[0] < left[1] - JOINT {
                break;
            }
            if !covers(top, x) {
                continue;
            }
            let mut bottom = None;
            for candidate in rows[..top_index].iter().rev() {
                steps += 1;
                if candidate[0] < left[1] - JOINT {
                    break;
                }
                if top[0] - candidate[0] > 2.0 && covers(candidate, x) {
                    bottom = Some(candidate);
                    break;
                }
            }
            let Some(bottom) = bottom else { continue };
            let reach = top[2].min(bottom[2]) + JOINT;
            let right = columns
                .iter()
                .skip_while(|right| right[0] <= x + 2.0)
                .take_while(|right| {
                    steps += 1;
                    right[0] <= reach
                })
                .find(|right| right[1] <= bottom[0] + JOINT && right[2] >= top[0] - JOINT);
            if let Some(right) = right {
                let cell = [x, bottom[0], right[0], top[0]];
                if !cells.iter().any(|c| same_box(*c, cell)) {
                    cells.push(cell);
                }
            }
        }
    }
    cells
}

/// `rules`'tan hücre kenarı olan kısımlar çıkarılır: tablo kenarlığı, üzerinde yazılacak bir
/// çizgi değildir. Kalan parçalar kural olarak kalır.
fn trim_edges(rules: &[[f64; 4]], cells: &[[f64; 4]]) -> Vec<[f64; 4]> {
    let mut out = Vec::with_capacity(rules.len());
    for rule in rules {
        let y = rule[1];
        let mut covered: Vec<[f64; 2]> = cells
            .iter()
            .filter(|c| (c[1] - y).abs() <= JOINT || (c[3] - y).abs() <= JOINT)
            .filter(|c| c[0] < rule[2] && c[2] > rule[0])
            .map(|c| [c[0] - JOINT, c[2] + JOINT])
            .collect();
        covered.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let mut x = rule[0];
        for [from, to] in covered {
            if from > x + 1.0 {
                out.push([x, y, from.min(rule[2]), y]);
            }
            x = x.max(to);
            if x >= rule[2] {
                break;
            }
        }
        if rule[2] - x > 1.0 {
            out.push([x, y, rule[2], y]);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Algılama
// ---------------------------------------------------------------------------

fn mid(r: [f64; 4]) -> f64 {
    (r[1] + r[3]) / 2.0
}

/// Etiketin kelimelerinden alan adı: harf, rakam, boşluk, `-` ve `/`; sondaki iki nokta yok.
fn clean(label: &str) -> String {
    let s: String = label
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '/' {
                c
            } else {
                ' '
            }
        })
        .collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    s.chars().take(40).collect::<String>().trim().to_string()
}

/// `rect`'teki alanın etiketi: aynı satırda solundaki kelimeler, yoksa hemen üstündekiler.
/// `right_of`: sağındaki kelimeler (onay kutuları).
fn label_for(words: &[Word], rect: [f64; 4], right_of: bool) -> String {
    let h = (rect[3] - rect[1]).max(8.0);
    let same_line = |w: &Word| (mid(w.rect) - mid(rect)).abs() < h.max(10.0) * 0.7;
    let mut side: Vec<&Word> = if right_of {
        words
            .iter()
            .filter(|w| same_line(w) && w.rect[0] >= rect[2] - 1.0 && w.rect[0] - rect[2] < 200.0)
            .collect()
    } else {
        words
            .iter()
            .filter(|w| same_line(w) && w.rect[2] <= rect[0] + 1.0 && rect[0] - w.rect[2] < 250.0)
            .collect()
    };
    side.sort_by(|a, b| a.rect[0].total_cmp(&b.rect[0]));
    if right_of {
        // Kutudan sonra gelen kelimeler, birkaç boşluktan geniş ilk aralığa kadar.
        let mut picked = Vec::new();
        let mut x = rect[2];
        for w in side {
            if w.rect[0] - x > 24.0 {
                break;
            }
            picked.push(w.text.as_str());
            x = w.rect[2];
        }
        return clean(&picked.join(" "));
    }
    // Alana en yakın biten kelimeler, geniş bir aralığa kadar geriye doğru.
    let mut picked: Vec<&str> = Vec::new();
    let mut x = rect[0];
    for w in side.iter().rev() {
        if x - w.rect[2] > 24.0 && !picked.is_empty() {
            break;
        }
        if w.text.chars().all(|c| c == '_') {
            break;
        }
        picked.insert(0, w.text.as_str());
        x = w.rect[0];
    }
    let l = clean(&picked.join(" "));
    if !l.is_empty() {
        return l;
    }
    // Üstte: alanın sol kısmının üzerindeki en yakın kelime satırı.
    let above: Vec<&Word> = words
        .iter()
        .filter(|w| {
            w.rect[1] >= rect[3] - 1.0
                && w.rect[1] - rect[3] < 24.0
                && w.rect[0] < rect[2]
                && w.rect[2] > rect[0] - 4.0
        })
        .collect();
    let Some(line) = above.iter().map(|w| mid(w.rect)).min_by(f64::total_cmp) else {
        return String::new();
    };
    let mut on: Vec<&&Word> = above
        .iter()
        .filter(|w| (mid(w.rect) - line).abs() < 3.0)
        .collect();
    on.sort_by(|a, b| a.rect[0].total_cmp(&b.rect[0]));
    clean(
        &on.iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Aynı boyda, birbirine değen iki ya da daha fazla hücrenin sıraları: (sıranın dikdörtgeni,
/// hücre sayısı). İki yeter: tarihler gün ve ay için ikişer hücreye bölünür. Hücreler kare ya da
/// eninden uzundur; daha geniş hücreli boş bir tablo comb değildir.
fn combs(boxes: &[[f64; 4]]) -> Vec<([f64; 4], usize)> {
    let cells: Vec<[f64; 4]> = boxes
        .iter()
        .copied()
        .filter(|r| {
            let (w, h) = (r[2] - r[0], r[3] - r[1]);
            (5.0..=30.0).contains(&w) && (8.0..=40.0).contains(&h) && w <= h * 1.2
        })
        .collect();
    // `b` bir sırada `a`'dan hemen sonra gelir: aynı boy, sol kenarı a'nın sağ kenarında.
    // Genişlik farkı 1 pt'ye kadar olabilir: kelime işlemci tablolarında baştaki ve sondaki
    // hücre dış kenarlığın kalınlığı kadar geniştir.
    let follows = |a: &[f64; 4], b: &[f64; 4]| {
        (b[0] - a[2]).abs() <= 1.0
            && (b[1] - a[1]).abs() <= 0.5
            && (b[3] - a[3]).abs() <= 0.5
            && ((b[2] - b[0]) - (a[2] - a[0])).abs() <= 1.0
    };
    let mut out = Vec::new();
    for first in &cells {
        if cells.iter().any(|a| follows(a, first)) {
            continue;
        }
        let (mut last, mut n) = (first, 1usize);
        // Her adım en az 4 pt sağa gider, bu yüzden biter; sınır güvenlik için.
        while n < cells.len() {
            let Some(b) = cells.iter().find(|b| follows(last, b)) else {
                break;
            };
            last = b;
            n += 1;
        }
        if n >= 2 {
            out.push(([first[0], first[1], last[2], first[3]], n));
        }
    }
    out
}

/// Bir sayfa için alan önerir. `existing` sayfanın widget dikdörtgenleridir (hiçbir aday
/// onlarla örtüşmez); adlar `taken`'a karşı tekildir.
pub fn detect(
    words: &[Word],
    shapes: &Shapes,
    existing: &[[f64; 4]],
    taken: &HashSet<String>,
) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let overlaps = |a: [f64; 4], b: [f64; 4]| {
        a[0] < b[2] - 1.0 && b[0] < a[2] - 1.0 && a[1] < b[3] - 1.0 && b[1] < a[3] - 1.0
    };
    let push = |kind: Kind, rect: [f64; 4], label: String, out: &mut Vec<Candidate>| {
        if existing
            .iter()
            .chain(out.iter().map(|c| &c.rect))
            .any(|e| overlaps(*e, rect))
        {
            return;
        }
        out.push(Candidate {
            kind,
            rect,
            name: label,
        });
    };
    // Alt çizgi olmayan metin (etiketler) ve alt çizgi dizileri (boşluklar).
    let (blanks, labels): (Vec<&Word>, Vec<&Word>) = words
        .iter()
        .partition(|w| w.text.len() >= 3 && w.text.chars().all(|c| c == '_'));
    let labels: Vec<Word> = labels.into_iter().cloned().collect();
    for b in blanks {
        let h = (b.rect[3] - b.rect[1]).max(10.0);
        let mut top = b.rect[1] + (h * 1.4).clamp(12.0, 22.0);
        // Alan hemen üstündeki kutuya taşmaz: kelime işlemci formlarında satır aralığı dardır
        // ve taşan alan kutunun alanını engellerdi.
        for r in &shapes.boxes {
            if r[0] < b.rect[2] && r[2] > b.rect[0] && r[1] >= b.rect[3] - 1.0 && r[1] < top {
                top = r[1].max(b.rect[3]);
            }
        }
        let rect = [b.rect[0], b.rect[1], b.rect[2], top];
        let l = label_for(&labels, rect, false);
        push(Kind::Text, rect, l, &mut out);
    }
    // Comb kutuları (posta kodu, tarih, IBAN): hücre sırası başına bir metin alanı; onay
    // kutularından önce bulunur ki hücreler tek tek onay kutusu olmasın.
    for (rect, cells) in combs(&shapes.boxes) {
        if labels.iter().any(|x| overlaps(x.rect, rect)) {
            continue;
        }
        let l = label_for(&labels, rect, false);
        push(Kind::Comb { cells }, rect, l, &mut out);
    }
    // Küçük kareler: sağlarındaki metinle adlandırılan onay kutuları.
    for r in &shapes.boxes {
        let (w, h) = (r[2] - r[0], r[3] - r[1]);
        if (5.0..=20.0).contains(&w) && (5.0..=20.0).contains(&h) && (w / h - 1.0).abs() < 0.25 {
            let l = label_for(&labels, *r, true);
            push(Kind::CheckBox, *r, l, &mut out);
        }
    }
    // İçine yazılacak kadar büyük boş kutular: içlerinde metin alanı.
    for r in &shapes.boxes {
        let (w, h) = (r[2] - r[0], r[3] - r[1]);
        let has_text = labels.iter().any(|x| overlaps(x.rect, *r));
        if w >= 40.0 && (12.0..=60.0).contains(&h) && !has_text {
            let inner = [r[0] + 1.0, r[1] + 1.0, r[2] - 1.0, r[3] - 1.0];
            let l = label_for(&labels, *r, false);
            push(Kind::Text, inner, l, &mut out);
        }
    }
    // Çizgiler: çizginin üzerinde duran metin alanı.
    for r in &shapes.rules {
        let w = r[2] - r[0];
        if w < 40.0 {
            continue;
        }
        // Metnin altını çizen çizgi (ya da üzerinde metin olan tablo kenarı) boşluk değildir.
        let rect = [r[0], r[1] + 1.0, r[2], r[1] + 17.0];
        if labels.iter().any(|x| overlaps(x.rect, rect)) {
            continue;
        }
        let l = label_for(&labels, rect, false);
        push(Kind::Text, rect, l, &mut out);
    }
    // Okuma sırası: yukarıdan aşağı, soldan sağa. Satır: ortası satırın ilk alanının ortasına
    // 8 pt'den yakın alanlar.
    out.sort_by(|a, b| mid(b.rect).total_cmp(&mid(a.rect)));
    let mut rows = Vec::with_capacity(out.len());
    let (mut row, mut start) = (0usize, f64::INFINITY);
    for c in &out {
        if start - mid(c.rect) > 8.0 {
            row += 1;
            start = mid(c.rect);
        }
        rows.push(row);
    }
    let mut keyed: Vec<(usize, Candidate)> = rows.into_iter().zip(out).collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.rect[0].total_cmp(&b.1.rect[0])));
    let mut out: Vec<Candidate> = keyed.into_iter().map(|(_, c)| c).collect();
    // Tekil adlar (Acrobat tekrarları numaralar); etiketsiz alanlar Text1, Check Box1, …
    let mut used: HashSet<String> = taken.clone();
    for c in &mut out {
        let base = if c.name.is_empty() {
            match c.kind {
                Kind::Text | Kind::Comb { .. } => "Text".to_string(),
                Kind::CheckBox => "Check Box".to_string(),
            }
        } else {
            c.name.clone()
        };
        let mut name = if c.name.is_empty() {
            format!("{base}1")
        } else {
            base.clone()
        };
        let mut n = 1;
        while used.contains(&name) {
            n += 1;
            name = format!("{base}{n}");
        }
        used.insert(name.clone());
        c.name = name;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::test_support::page_pdf as pdf;
    use super::*;

    fn word(t: &str, x0: f64, y0: f64, x1: f64) -> Word {
        Word {
            text: t.into(),
            rect: [x0, y0, x1, y0 + 10.0],
        }
    }

    fn taken(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn detection_finds_blanks_rules_boxes_and_names_them() {
        let words = vec![
            word("Name:", 50.0, 700.0, 80.0),
            word("______________", 85.0, 699.0, 250.0),
            word("Date", 300.0, 700.0, 325.0),
            word("of", 328.0, 700.0, 338.0),
            word("birth", 341.0, 700.0, 365.0),
            word("I", 72.0, 602.0, 75.0),
            word("agree", 78.0, 602.0, 105.0),
            word("Comments", 50.0, 560.0, 100.0),
            word("Heading", 50.0, 750.0, 100.0),
        ];
        let shapes = Shapes {
            boxes: vec![
                [56.0, 600.0, 68.0, 612.0],
                [50.0, 500.0, 300.0, 540.0],
                [40.0, 20.0, 560.0, 780.0],
            ],
            rules: vec![[370.0, 699.0, 500.0, 699.0], [50.0, 748.0, 100.0, 748.0]],
        };
        let found = detect(&words, &shapes, &[], &taken(&["Comments"]));
        let names: Vec<(&str, &Kind)> = found.iter().map(|c| (c.name.as_str(), &c.kind)).collect();
        assert_eq!(
            names,
            [
                ("Name", &Kind::Text),
                ("Date of birth", &Kind::Text),
                ("I agree", &Kind::CheckBox),
                ("Comments2", &Kind::Text)
            ],
            "{found:?}"
        );
        // Orada zaten bir alan var: yeni bir şey yok.
        let existing: Vec<[f64; 4]> = found.iter().map(|c| c.rect).collect();
        let again = detect(&words, &shapes, &existing, &HashSet::new());
        assert!(again.is_empty(), "{again:?}");
    }

    #[test]
    fn detection_makes_a_row_of_touching_cells_one_comb_field() {
        let words = vec![
            word("Postcode", 40.0, 205.0, 85.0),
            word("A", 88.0, 205.0, 95.0),
            word("Postcode", 40.0, 105.0, 85.0),
            word("B", 88.0, 105.0, 95.0),
            word("Yes", 70.0, 41.0, 90.0),
        ];
        let row = |n: usize, x: f64, y: f64, cw: f64, ch: f64| {
            (0..n).map(move |i| [x + i as f64 * cw, y, x + (i + 1) as f64 * cw, y + ch])
        };
        // 18 × 22 hücreler (onay kutusu için fazla uzun), sağdan sola çizilmiş 16 × 20 hücreler
        // (her biri onay kutusu boyunda) ve tek bir kare.
        let mut boxes: Vec<[f64; 4]> = row(5, 120.0, 200.0, 18.0, 22.0).collect();
        boxes.extend(row(10, 120.0, 100.0, 16.0, 20.0).rev());
        boxes.push([50.0, 40.0, 62.0, 52.0]);
        let found = detect(
            &words,
            &Shapes {
                boxes,
                rules: Vec::new(),
            },
            &[],
            &HashSet::new(),
        );
        let got: Vec<(&str, &Kind, [f64; 4])> = found
            .iter()
            .map(|c| (c.name.as_str(), &c.kind, c.rect))
            .collect();
        assert_eq!(
            got,
            [
                (
                    "Postcode A",
                    &Kind::Comb { cells: 5 },
                    [120.0, 200.0, 210.0, 222.0]
                ),
                (
                    "Postcode B",
                    &Kind::Comb { cells: 10 },
                    [120.0, 100.0, 280.0, 120.0]
                ),
                ("Yes", &Kind::CheckBox, [50.0, 40.0, 62.0, 52.0]),
            ],
            "{found:?}"
        );
        // Aralarında boşluk olan kareler ve farklı boyda değen kareler onay kutusu kalır.
        let apart = Shapes {
            boxes: vec![
                [50.0, 40.0, 62.0, 52.0],
                [66.0, 40.0, 78.0, 52.0],
                [80.0, 40.0, 92.0, 52.0],
                [92.0, 40.0, 102.0, 52.0],
            ],
            rules: Vec::new(),
        };
        let kinds: Vec<Kind> = detect(&[], &apart, &[], &HashSet::new())
            .into_iter()
            .map(|c| c.kind)
            .collect();
        assert_eq!(kinds, [Kind::CheckBox; 4]);
        // Eninden kısa hücreli boş bir tablo (yoklama çizelgesi) comb değildir.
        let grid = |cw: f64, ch: f64| {
            (0..3).flat_map(move |r| {
                (0..3).map(move |c| {
                    [
                        100.0 + c as f64 * cw,
                        300.0 + r as f64 * ch,
                        100.0 + (c + 1) as f64 * cw,
                        300.0 + (r + 1) as f64 * ch,
                    ]
                })
            })
        };
        for (cw, ch) in [(24.0, 18.0), (28.0, 20.0)] {
            let shapes = Shapes {
                boxes: grid(cw, ch).collect(),
                rules: Vec::new(),
            };
            let table = detect(&[], &shapes, &[], &HashSet::new());
            assert!(table.is_empty(), "{cw} x {ch}: {table:?}");
        }
    }

    #[test]
    fn separate_lines_close_into_cells_and_their_edges_stop_being_rules() {
        // 3 × 2 ızgara: çizgiler köşelerde yarım nokta taşar ya da eksik kalır, üst kenar iki
        // parça, alt kenar ızgaranın sağına 60 pt uzar.
        let horizontal = [
            [49.6, 140.0, 110.0, 140.0],
            [110.0, 140.0, 170.4, 140.0],
            [50.0, 120.0, 170.0, 120.0],
            [49.6, 100.0, 230.0, 100.0],
        ];
        let vertical = [
            [50.0, 99.6, 50.0, 140.4],
            [90.0, 100.0, 90.0, 140.0],
            [130.0, 100.4, 130.0, 139.6],
            [170.0, 99.6, 170.0, 140.4],
        ];
        let cells = cells_from_lines(&horizontal, &vertical);
        let mut rounded: Vec<[f64; 4]> = cells.iter().map(|c| c.map(|v| v.round())).collect();
        rounded.sort_by(|a, b| a[1].total_cmp(&b[1]).then(a[0].total_cmp(&b[0])));
        assert_eq!(
            rounded,
            [
                [50.0, 100.0, 90.0, 120.0],
                [90.0, 100.0, 130.0, 120.0],
                [130.0, 100.0, 170.0, 120.0],
                [50.0, 120.0, 90.0, 140.0],
                [90.0, 120.0, 130.0, 140.0],
                [130.0, 120.0, 170.0, 140.0],
            ]
        );
        // Kenarlar kural olmaktan çıkar; alt kenarın ızgara dışındaki parçası kalır.
        let rules = trim_edges(&horizontal, &cells);
        assert_eq!(rules.len(), 1, "{rules:?}");
        assert!(
            (rules[0][0] - 171.5).abs() < 0.01 && rules[0][2] == 230.0,
            "{rules:?}"
        );
        // Dikey çizgi alt kenara ulaşmıyorsa hücre yoktur.
        let open = [[50.0, 130.0, 50.0, 140.0], [90.0, 100.0, 90.0, 140.0]];
        assert!(cells_from_lines(&horizontal[..3], &open)
            .iter()
            .all(|c| c[0] > 50.5));
    }

    /// LibreOffice Writer çıktısındaki konumlar: "Bölüm" satırı adres kutusunun hemen altında.
    /// Alt çizgi alanı kutuya taşıp onun yerini almaz.
    #[test]
    fn a_blank_right_under_a_box_leaves_the_box_its_field() {
        let words = vec![
            Word {
                text: "Adres".into(),
                rect: [56.8, 699.0, 88.1, 712.4],
            },
            Word {
                text: "Bölüm:".into(),
                rect: [56.8, 633.6, 94.1, 647.0],
            },
            Word {
                text: "______________________".into(),
                rect: [97.2, 633.6, 244.2, 647.0],
            },
        ];
        let shapes = Shapes {
            boxes: vec![[56.7, 647.289, 361.0, 689.089]],
            rules: Vec::new(),
        };
        let found = detect(&words, &shapes, &[], &HashSet::new());
        let got: Vec<(&str, [f64; 4])> = found
            .iter()
            .map(|c| (c.name.as_str(), c.rect.map(|v| (v * 10.0).round() / 10.0)))
            .collect();
        assert_eq!(
            got,
            [
                ("Adres", [57.7, 648.3, 360.0, 688.1]),
                ("Bölüm", [97.2, 633.6, 244.2, 647.3]),
            ],
            "{found:?}"
        );
    }

    // --- PDFium girdisi ------------------------------------------------------------------

    fn read(bytes: &[u8]) -> PageInput {
        let pdfium = crate::pdf_engine::get_pdfium().unwrap();
        let document = pdfium.load_pdf_from_byte_slice(bytes, None).unwrap();
        let page = document.pages().get(0).unwrap();
        read_page(&page).unwrap()
    }

    fn shapes(content: &str) -> Shapes {
        read(&pdf(content, None)).shapes
    }

    fn round(shapes: Shapes) -> Shapes {
        let r = |v: [f64; 4]| v.map(|x| (x * 100.0).round() / 100.0);
        Shapes {
            boxes: shapes.boxes.into_iter().map(r).collect(),
            rules: shapes.rules.into_iter().map(r).collect(),
        }
    }

    #[test]
    fn page_shapes_reads_boxes_and_rules() {
        let shapes =
            shapes("q 2 0 0 2 0 0 cm 10 10 6 6 re S 20 50 m 120 50 l S 0 0 m 5 5 l S 30 300 100 0.5 re f Q");
        assert_eq!(shapes.boxes, [[20.0, 20.0, 32.0, 32.0]]);
        assert_eq!(
            shapes.rules,
            [[40.0, 100.0, 240.0, 100.0], [60.0, 600.5, 260.0, 600.5]]
        );
    }

    #[test]
    fn page_shapes_reads_rectangle_shorthand_and_outline_alike() {
        let shorthand = shapes("50 100 120 40 re S");
        let outlined = shapes("50 100 m 170 100 l 170 140 l 50 140 l h S");
        assert_eq!(outlined, shorthand);
        let fields = detect(&[], &outlined, &[], &HashSet::new());
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].rect, [51.0, 101.0, 169.0, 139.0]);
    }

    #[test]
    fn page_shapes_accepts_rectangle_closure_and_paint_variants() {
        for end in [
            "h S",
            "s",
            "50 100 l S",
            "f",
            "F",
            "f*",
            "B",
            "B*",
            "b",
            "b*",
        ] {
            let shapes = shapes(&format!("50 100 m 170 100 l 170 140 l 50 140 l {end}"));
            assert_eq!(shapes.boxes, [[50.0, 100.0, 170.0, 140.0]], "{end}");
            assert!(shapes.rules.is_empty(), "{end}: {:?}", shapes.rules);
        }
    }

    /// #2942: her dikey kenarının ortasında (ya da her kenarında) noktası olan hücre göründüğü
    /// dikdörtgendir; kenar boyunca geri dönen nokta değildir.
    #[test]
    fn page_shapes_reads_rectangles_with_points_along_their_sides() {
        for content in [
            "100 100 m 111.1 100 l 111.1 107.35 l 111.1 114.7 l 100 114.7 l 100 107.35 l h f",
            "100 100 m 105 100 l 111.1 100 l 111.1 107.35 l 111.1 114.7 l 105 114.7 l 100 114.7 l 100 107.35 l h S",
            "100 107.35 m 100 100 l 111.1 100 l 111.1 114.7 l 100 114.7 l h f",
        ] {
            assert_eq!(
                round(shapes(content)).boxes,
                [[100.0, 100.0, 111.1, 114.7]],
                "{content}"
            );
        }
        // Kenarın dışına taşıp geri dönen nokta yolu dikdörtgen yapmaz; ama çizgileri yine de
        // görünen kutuyu kapatır, o kutu çizgilerden kurulur (PdfCraft burada kutu bulmaz).
        let spike = "100 100 m 130 100 l 111.1 100 l 111.1 114.7 l 100 114.7 l h S";
        let path = DetectionPath {
            points: vec![
                [100.0, 100.0],
                [130.0, 100.0],
                [111.1, 100.0],
                [111.1, 114.7],
                [100.0, 114.7],
            ],
            closed: true,
            ..DetectionPath::default()
        };
        assert_eq!(path.rectangle(false), None);
        assert_eq!(
            round(shapes(spike)).boxes,
            [[100.0, 100.0, 111.1, 114.7]],
            "{spike}"
        );
        // Kenarların dışında altıncı bir nokta: çapraz kenarlar, kutu yok.
        let off_side = "100 100 m 111.1 100 l 115 107 l 111.1 114.7 l 100 114.7 l 100 107.35 l h S";
        assert!(shapes(off_side).boxes.is_empty(), "{off_side}");
    }

    #[test]
    fn page_shapes_rejects_open_diagonal_curved_and_complex_rectangles() {
        for content in [
            "50 100 m 170 100 l 170 140 l 50 140 l S",
            "50 100 m 170 110 l 170 150 l 50 140 l h S",
            "50 100 m 170 100 l 170 140 l 100 150 80 150 50 140 c h S",
            "50 100 m 170 100 l 170 140 l h 50 140 l f",
            "50 100 m 170 100 l 50 100 l 50 140 l h S",
        ] {
            assert!(shapes(content).boxes.is_empty(), "{content}");
        }
    }

    #[test]
    fn page_shapes_keeps_subpaths_apart_and_skips_unpainted_paths() {
        let found = shapes(
            "50 100 120 40 re 200 100 m 320 100 l 320 140 l 200 140 l h 50 400 m 170 400 l S",
        );
        assert_eq!(
            found.boxes,
            [[50.0, 100.0, 170.0, 140.0], [200.0, 100.0, 320.0, 140.0]]
        );
        assert_eq!(found.rules, [[50.0, 400.0, 170.0, 400.0]]);
        assert_eq!(
            shapes("50 100 m 170 100 l 170 140 l 50 140 l h W n"),
            Shapes::default()
        );
    }

    #[test]
    fn page_shapes_transforms_paths_including_form_xobjects() {
        let scaled = shapes(
            "q 2 0 0 2 0 0 cm 50 100 m 170 100 l 170 140 l 50 140 l h S Q 10 20 m 100 20 l S",
        );
        assert_eq!(scaled.boxes, [[100.0, 200.0, 340.0, 280.0]]);
        assert_eq!(scaled.rules, [[10.0, 20.0, 100.0, 20.0]]);
        let rotated = shapes("0 1 -1 0 300 0 cm 50 100 m 170 100 l 170 140 l 50 140 l h S");
        assert_eq!(round(rotated).boxes, [[160.0, 50.0, 200.0, 170.0]]);
        // Form XObject: önce kendi /Matrix'i, sonra çağrıldığı yerdeki CTM.
        let nested = read(&pdf(
            "q 1 0 0 1 100 0 cm /Fm Do Q",
            Some((
                "50 100 120 40 re S 10 20 m 100 20 l S",
                [2.0, 0.0, 0.0, 2.0, 0.0, 0.0],
            )),
        ))
        .shapes;
        assert_eq!(nested.boxes, [[200.0, 200.0, 440.0, 280.0]]);
        assert_eq!(nested.rules, [[120.0, 40.0, 300.0, 40.0]]);
    }

    #[test]
    fn page_shapes_closes_only_the_last_stroked_subpath_but_every_filled_one() {
        let paths = "50 100 m 170 100 l 170 140 l 50 140 l 200 100 m 320 100 l 320 140 l 200 140 l";
        let stroked = shapes(&format!("{paths} s"));
        assert_eq!(stroked.boxes, [[200.0, 100.0, 320.0, 140.0]]);
        assert_eq!(
            stroked.rules,
            [[50.0, 100.0, 170.0, 100.0], [50.0, 140.0, 170.0, 140.0]]
        );
        let filled = shapes(&format!("{paths} f"));
        assert_eq!(
            filled.boxes,
            [[50.0, 100.0, 170.0, 140.0], [200.0, 100.0, 320.0, 140.0]]
        );
        assert!(filled.rules.is_empty());
    }

    /// LibreOffice Writer'ın bir HTML tablosundan ürettiği çizgiler (koordinatlar gerçek
    /// çıktıdan): her kenarlık ayrı bir path, dikeyler köşelerde taşar, ilk ve son hücre
    /// ötekilerden 0,8 pt geniş.
    #[test]
    fn a_word_processor_table_form_reads_into_comb_and_box_fields() {
        let mut content = String::from(
            "BT /F1 10 Tf 56.8 766 Td (Ad Soyad: ______________________________) Tj ET \
             BT /F1 10 Tf 56.8 739 Td (TC Kimlik No) Tj ET \
             BT /F1 10 Tf 56.8 693 Td (Adres) Tj ET \
             BT /F1 10 Tf 56.8 620 Td (Bolum: ______________________) Tj ET ",
        );
        let line = |x0: f64, y0: f64, x1: f64, y1: f64| format!("{x0} {y0} m {x1} {y1} l S ");
        content += &line(56.35, 730.689, 190.6, 730.689);
        content += &line(56.35, 712.689, 190.6, 712.689);
        for x in [
            56.7, 69.5, 81.5, 93.5, 105.5, 117.5, 129.5, 141.5, 153.5, 165.5, 177.5, 190.2,
        ] {
            content += &line(x, 731.089, x, 712.339);
        }
        content += &line(56.35, 689.089, 361.4, 689.089);
        content += &line(56.35, 647.289, 361.4, 647.289);
        content += &line(56.7, 689.489, 56.7, 646.939);
        content += &line(361.0, 689.489, 361.0, 646.939);
        let input = read(&pdf(&content, None));
        let found = detect(
            &input.words,
            &input.shapes,
            &input.existing,
            &HashSet::new(),
        );
        let got: Vec<(&str, Kind)> = found.iter().map(|c| (c.name.as_str(), c.kind)).collect();
        assert_eq!(
            got,
            [
                ("Ad Soyad", Kind::Text),
                ("TC Kimlik No", Kind::Comb { cells: 11 }),
                ("Adres", Kind::Text),
                ("Bolum", Kind::Text),
            ],
            "{found:?}"
        );
        let address = found[2].rect.map(|v| v.round());
        assert_eq!(address, [58.0, 648.0, 360.0, 688.0]);
    }

    #[test]
    fn a_row_box_split_by_vertical_lines_becomes_cells() {
        // Tek bir satır dikdörtgeni ve hücre ayraçları ayrı dikey çizgiler.
        let mut content =
            String::from("BT /F1 10 Tf 40 505 Td (Postcode) Tj ET 120 500 90 22 re S ");
        for x in [138, 156, 174, 192] {
            content += &format!("{x} 500 m {x} 522 l S ");
        }
        let input = read(&pdf(&content, None));
        let found = detect(
            &input.words,
            &input.shapes,
            &input.existing,
            &HashSet::new(),
        );
        let got: Vec<(&str, Kind, [f64; 4])> = found
            .iter()
            .map(|c| (c.name.as_str(), c.kind, c.rect.map(|v| v.round())))
            .collect();
        assert_eq!(
            got,
            [(
                "Postcode",
                Kind::Comb { cells: 5 },
                [120.0, 500.0, 210.0, 522.0]
            )],
            "{found:?}"
        );
    }

    #[test]
    fn a_printed_form_reads_into_named_fields() {
        let input = read(&pdf(
            "BT /F1 10 Tf 50 700 Td (Name: ______________________) Tj ET \
             BT /F1 10 Tf 50 650 Td (City) Tj ET 90 648 m 300 648 l S \
             56 600 10 10 re S BT /F1 10 Tf 72 601 Td (I agree) Tj ET",
            None,
        ));
        assert!(
            input.words.iter().any(|w| w.text == "Name:"),
            "underscores split from the label: {:?}",
            input.words
        );
        let found = detect(
            &input.words,
            &input.shapes,
            &input.existing,
            &HashSet::new(),
        );
        let names: Vec<(&str, &Kind)> = found.iter().map(|c| (c.name.as_str(), &c.kind)).collect();
        assert_eq!(
            names,
            [
                ("Name", &Kind::Text),
                ("City", &Kind::Text),
                ("I agree", &Kind::CheckBox)
            ],
            "{found:?}"
        );
    }
}
