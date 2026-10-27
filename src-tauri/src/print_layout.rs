// Copyright (c) 2026 ArtCraft Team and the PdfCraft contributors.
// Licensed under the MIT License; see src-tauri/licenses/pdfcraft.txt.
//
// Ported from PdfCraft 0.14.0, crates/print/src/lib.rs (`layout` and the types, paper list
// and CUPS media names it needs) and crates/content/src/lib.rs (`Matrix`, the parts used here);
// MIT OR Apache-2.0, used under MIT. Changes from the original: the shipping-label layout, the
// Comments & Forms and print-as-image settings, `impose` and the spooler are left out (they
// work on PdfCraft's own PDF model; `print_imposition` builds the sheets with PDFium instead);
// `PrintError` reduced to `LayoutError` with Turkish messages.

//! Yazdırma tabakalarının yerleşimi: her kaynak sayfanın hangi tabakada, hangi matrisle
//! duracağı. Acrobat'ın Yazdır penceresindeki gibi: Boyut (sığdır, gerçek boyut, küçült, özel
//! ölçek), tabaka başına birden çok sayfa (ızgara, dört sıra ve kes-diz, çerçeve, otomatik
//! döndürme), kitapçık (iki yüz, yalnız ön ya da arka; sol ya da sağ ciltleme) ve poster
//! (bindirme payı, kesim işaretleri). Saf hesap: PDF okumaz ya da yazmaz.

/// A PDF transformation matrix `[a b c d e f]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix(pub [f64; 6]);

impl Default for Matrix {
    fn default() -> Self {
        Matrix::IDENTITY
    }
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// `self` then `then` (`self × then` in PDF's row-vector convention).
    pub fn then(&self, then: &Matrix) -> Matrix {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = then.0;
        Matrix([
            a * a2 + b * c2,
            a * b2 + b * d2,
            c * a2 + d * c2,
            c * b2 + d * d2,
            e * a2 + f * c2 + e2,
            e * b2 + f * d2 + f2,
        ])
    }

    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }

    /// The bounding box of a rectangle `[x0 y0 x1 y1]` after this transform.
    pub fn bbox(&self, r: [f64; 4]) -> [f64; 4] {
        let pts = [
            self.apply(r[0], r[1]),
            self.apply(r[2], r[1]),
            self.apply(r[0], r[3]),
            self.apply(r[2], r[3]),
        ];
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for (x, y) in pts {
            b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
        }
        b
    }
}

/// Why sheets cannot be laid out.
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutError {
    NoPages,
    Invalid(String),
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutError::NoPages => f.write_str("Yazdırılacak sayfa yok"),
            LayoutError::Invalid(message) => f.write_str(message),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Auto,
    Portrait,
    Landscape,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum SizeMode {
    #[default]
    Fit,
    Actual,
    Shrink,
    /// Percent.
    Custom(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PageOrder {
    #[default]
    Horizontal,
    HorizontalReversed,
    Vertical,
    VerticalReversed,
    /// Single-sided sheets: cut into cell piles, then stack them in row-major order.
    CutStack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BookletSubset {
    #[default]
    BothSides,
    FrontOnly,
    BackOnly,
}

/// How a booklet goes through the printer (#2707): it decides two-sided printing itself, as
/// Acrobat does (its Print dialog hides Two-sided while Booklet is chosen; Using Acrobat X Pro,
/// Print dialog box options: "Acrobat automatically enables duplex printing, if available, for
/// booklet printing"). Both sides flip on the short edge, the fold of a 2-up sheet turned
/// sideways; front side only or back side only is one pass of a manual duplex and prints
/// single-sided.
pub fn booklet_duplex(subset: BookletSubset) -> Duplex {
    match subset {
        BookletSubset::BothSides => Duplex::ShortEdge,
        BookletSubset::FrontOnly | BookletSubset::BackOnly => Duplex::Off,
    }
}

/// Two-sided printing as the spooler is asked for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Duplex {
    Off,
    ShortEdge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Binding {
    #[default]
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layout {
    Size(SizeMode),
    Multiple {
        cols: usize,
        rows: usize,
        /// Source pages on each sheet, at most `cols × rows`: three per sheet uses a 2 × 2 grid
        /// with one cell left blank.
        per_sheet: usize,
        order: PageOrder,
        border: bool,
        auto_rotate: bool,
    },
    Booklet {
        subset: BookletSubset,
        binding: Binding,
    },
    /// Tile scale (percent), overlap (points), cut marks.
    Poster {
        scale: f64,
        overlap: f64,
        cut_marks: bool,
    },
}

impl Default for Layout {
    fn default() -> Self {
        Layout::Size(SizeMode::Fit)
    }
}

impl Layout {
    /// Acrobat's pages-per-sheet choices as a grid (columns × rows on a portrait sheet).
    pub fn multiple(per_sheet: usize) -> Layout {
        let (cols, rows) = match per_sheet {
            2 => (1, 2),
            4 => (2, 2),
            6 => (2, 3),
            9 => (3, 3),
            16 => (4, 4),
            n => {
                let c = (n as f64).sqrt().ceil().max(1.0) as usize;
                (c, n.div_ceil(c).max(1))
            }
        };
        Layout::Multiple {
            cols,
            rows,
            per_sheet: per_sheet.max(1),
            order: PageOrder::Horizontal,
            border: false,
            auto_rotate: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Source pages (0-based) in print order.
    pub pages: Vec<usize>,
    /// Paper (width, height) in points, portrait.
    pub paper: (f64, f64),
    pub orientation: Orientation,
    pub layout: Layout,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            pages: Vec::new(),
            paper: PAPERS[0].1,
            orientation: Orientation::Auto,
            layout: Layout::default(),
        }
    }
}

/// Common paper sizes (points, portrait).
pub const PAPERS: [(&str, (f64, f64)); 21] = [
    ("US Letter", (612.0, 792.0)),
    ("US Legal", (612.0, 1008.0)),
    ("Tabloid", (792.0, 1224.0)),
    ("A3", (841.89, 1190.55)),
    ("A4", (595.28, 841.89)),
    ("A5", (419.53, 595.28)),
    ("A6", (297.64, 419.53)),
    ("ARCH A (9 x 12 in)", (648.0, 864.0)),
    ("ARCH B (12 x 18 in)", (864.0, 1296.0)),
    ("ARCH C (18 x 24 in)", (1296.0, 1728.0)),
    ("ARCH D (24 x 36 in)", (1728.0, 2592.0)),
    ("ARCH E1 (30 x 42 in)", (2160.0, 3024.0)),
    ("ARCH E (36 x 48 in)", (2592.0, 3456.0)),
    ("ANSI C (17 x 22 in)", (1224.0, 1584.0)),
    ("ANSI D (22 x 34 in)", (1584.0, 2448.0)),
    ("ANSI E (34 x 44 in)", (2448.0, 3168.0)),
    ("A2", (1190.55, 1683.78)),
    ("A1", (1683.78, 2383.94)),
    ("A0", (2383.94, 3370.39)),
    ("4 x 6 in label (101.6 x 152.4 mm)", (288.0, 432.0)),
    ("US #10 Envelope (4.125 x 9.5 in)", (297.0, 684.0)),
];

/// `PAPERS` index of the 4 × 6 in shipping label.
pub const LABEL_PAPER: usize = 19;

/// The CUPS media name for `PAPERS[index]`, so the spooler feeds the sheet the PDF was laid out
/// on rather than the queue's default; `None` only past the end of `PAPERS`. The common sizes
/// use their PPD keywords (`w288h432` is the usual 4×6 label, `Env10` the US #10 envelope).
/// The rest use PWG 5101.1 names (ARCH E1 is its `wide-format` size), spelled exactly as the
/// spec and the IANA registry have them: a driverless printer may take only the names it
/// advertises. They carry their dimensions, so CUPS feeds the PPD size of those dimensions
/// whatever the driver calls it (`ARCHD`, `AnsiD`) (#3752).
pub fn cups_media(index: usize) -> Option<&'static str> {
    Some(match PAPERS.get(index)?.0 {
        "US Letter" => "Letter",
        "US Legal" => "Legal",
        "Tabloid" => "Tabloid",
        "A3" => "A3",
        "A4" => "A4",
        "A5" => "A5",
        "A6" => "iso_a6_105x148mm",
        "ARCH A (9 x 12 in)" => "na_arch-a_9x12in",
        "ARCH B (12 x 18 in)" => "na_arch-b_12x18in",
        "ARCH C (18 x 24 in)" => "na_arch-c_18x24in",
        "ARCH D (24 x 36 in)" => "na_arch-d_24x36in",
        "ARCH E1 (30 x 42 in)" => "na_wide-format_30x42in",
        "ARCH E (36 x 48 in)" => "na_arch-e_36x48in",
        "ANSI C (17 x 22 in)" => "na_c_17x22in",
        "ANSI D (22 x 34 in)" => "na_d_22x34in",
        "ANSI E (34 x 44 in)" => "na_e_34x44in",
        "A2" => "iso_a2_420x594mm",
        "A1" => "iso_a1_594x841mm",
        "A0" => "iso_a0_841x1189mm",
        "US #10 Envelope (4.125 x 9.5 in)" => "Env10",
        _ if index == LABEL_PAPER => "w288h432",
        // A size added to PAPERS without a name here: its tests fail, and meanwhile the queue's
        // paper stands, as it did for every size before #1583.
        _ => return None,
    })
}

/// Match a displayed PDF page to a standard sheet, independent of orientation.
/// Half a point allows rounded metric PDF dimensions without confusing distinct sizes.
pub fn matching_paper(size: (f64, f64)) -> Option<usize> {
    if !size.0.is_finite() || !size.1.is_finite() {
        return None;
    }
    let size = (size.0.min(size.1), size.0.max(size.1));
    PAPERS
        .iter()
        .position(|(_, p)| (p.0 - size.0).abs() <= 0.5 && (p.1 - size.1).abs() <= 0.5)
}

/// The unprintable margin assumed around the sheet for Fit, Multiple and Booklet.
pub const MARGIN: f64 = 18.0;

/// One page as placed on a sheet: the source page, the matrix from its *display space* to the
/// sheet, and the visible part of it in display space (clipping).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub page: usize,
    pub matrix: Matrix,
    pub clip: [f64; 4],
}

/// One output sheet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sheet {
    pub size: (f64, f64),
    pub placed: Vec<Placement>,
    /// Page borders (Multiple) and cut marks (Poster), in sheet space: rectangles and lines.
    pub borders: Vec<[f64; 4]>,
    pub lines: Vec<[f64; 4]>,
}

fn oriented(paper: (f64, f64), o: Orientation, landscape_wanted: bool) -> (f64, f64) {
    let (w, h) = (paper.0.min(paper.1), paper.0.max(paper.1));
    match o {
        Orientation::Portrait => (w, h),
        Orientation::Landscape => (h, w),
        Orientation::Auto => {
            if landscape_wanted {
                (h, w)
            } else {
                (w, h)
            }
        }
    }
}

/// Place a page of display size `(dw, dh)` inside `cell` (sheet space), scaled by `s`
/// (centred), optionally rotated 90° (counter-clockwise).
fn place_in(page: usize, (dw, dh): (f64, f64), cell: [f64; 4], s: f64, rotate: bool) -> Placement {
    let (cw, ch) = (cell[2] - cell[0], cell[3] - cell[1]);
    let (pw, ph) = if rotate {
        (dh * s, dw * s)
    } else {
        (dw * s, dh * s)
    };
    let (x, y) = (cell[0] + (cw - pw) / 2.0, cell[1] + (ch - ph) / 2.0);
    let matrix = if rotate {
        Matrix([0.0, s, -s, 0.0, x + pw, y])
    } else {
        Matrix([s, 0.0, 0.0, s, x, y])
    };
    Placement {
        page,
        matrix,
        clip: [0.0, 0.0, dw, dh],
    }
}

fn fit_scale((dw, dh): (f64, f64), (cw, ch): (f64, f64)) -> f64 {
    if dw <= 0.0 || dh <= 0.0 {
        1.0
    } else {
        (cw / dw).min(ch / dh)
    }
}

/// Lay out the sheets for `settings` given each source page's display size.
pub fn layout(sizes: &[(f64, f64)], settings: &Settings) -> Result<Vec<Sheet>, LayoutError> {
    let pages: Vec<usize> = settings
        .pages
        .iter()
        .copied()
        .filter(|p| *p < sizes.len())
        .collect();
    if pages.is_empty() {
        return Err(LayoutError::NoPages);
    }
    let paper = settings.paper;
    if !(paper.0 >= 72.0 && paper.1 >= 72.0 && paper.0.is_finite() && paper.1.is_finite()) {
        return Err(LayoutError::Invalid("Kağıt boyutu geçersiz".into()));
    }
    let mut sheets = Vec::new();
    match settings.layout {
        Layout::Size(mode) => {
            for &p in &pages {
                let d = sizes[p];
                let size = oriented(paper, settings.orientation, d.0 > d.1);
                let fit = fit_scale(d, (size.0 - 2.0 * MARGIN, size.1 - 2.0 * MARGIN));
                let s = match mode {
                    SizeMode::Fit => fit,
                    SizeMode::Actual => 1.0,
                    SizeMode::Shrink => fit.min(1.0),
                    SizeMode::Custom(pct) if pct.is_finite() && pct > 0.0 => pct / 100.0,
                    SizeMode::Custom(_) => {
                        return Err(LayoutError::Invalid("Ölçek %0'dan büyük olmalı".into()))
                    }
                };
                sheets.push(Sheet {
                    size,
                    placed: vec![place_in(p, d, [0.0, 0.0, size.0, size.1], s, false)],
                    ..Sheet::default()
                });
            }
        }
        Layout::Multiple {
            cols,
            rows,
            per_sheet,
            order,
            border,
            auto_rotate,
        } => {
            let cells = cols.checked_mul(rows).filter(|&n| n > 0 && n <= 256);
            let Some(cells) = cells else {
                return Err(LayoutError::Invalid(
                    "Tabaka başına sayfa sayısı geçersiz".into(),
                ));
            };
            // The grid can have more cells than the pages asked for on each sheet; the rest stay blank.
            let per_sheet = per_sheet.min(cells);
            if per_sheet == 0 {
                return Err(LayoutError::Invalid(
                    "Tabaka başına sayfa sayısı geçersiz".into(),
                ));
            }
            // A grid with more columns than rows wants a landscape sheet, and the reverse.
            let first = pages
                .first()
                .and_then(|&p| sizes.get(p))
                .copied()
                .ok_or(LayoutError::NoPages)?;
            let page_landscape = first.0 > first.1;
            let (gc, gr, sheet_landscape) = if cols == rows {
                (cols, rows, page_landscape)
            } else if settings.orientation == Orientation::Landscape
                || (settings.orientation == Orientation::Auto && cols < rows)
            {
                // 2 per sheet (1 × 2 on portrait) prints side by side on landscape paper.
                (rows, cols, true)
            } else {
                (cols, rows, false)
            };
            let size = oriented(paper, settings.orientation, sheet_landscape);
            let gap = 6.0;
            let (aw, ah) = (size.0 - 2.0 * MARGIN, size.1 - 2.0 * MARGIN);
            let (cw, ch) = (
                (aw - gap * (gc - 1) as f64) / gc as f64,
                (ah - gap * (gr - 1) as f64) / gr as f64,
            );
            if cw <= 0.0 || ch <= 0.0 {
                return Err(LayoutError::Invalid(
                    "Sayfa ızgarası kağıda sığmıyor".into(),
                ));
            }
            let sheet_count = pages.len().div_ceil(per_sheet);
            for sheet_index in 0..sheet_count {
                let mut sheet = Sheet {
                    size,
                    ..Sheet::default()
                };
                if order == PageOrder::CutStack {
                    // Marks align across every sheet, including unoccupied cells. Cut in the
                    // middle of each gutter; marks stay in the printable area, clear of cells.
                    for col in 1..gc {
                        let x = MARGIN + col as f64 * (cw + gap) - gap / 2.0;
                        sheet.lines.extend([
                            [x, MARGIN, x, MARGIN + 8.0],
                            [x, size.1 - MARGIN - 8.0, x, size.1 - MARGIN],
                        ]);
                    }
                    for row in 1..gr {
                        let y = size.1 - MARGIN - row as f64 * (ch + gap) + gap / 2.0;
                        sheet.lines.extend([
                            [MARGIN, y, MARGIN + 8.0, y],
                            [size.0 - MARGIN - 8.0, y, size.0 - MARGIN, y],
                        ]);
                    }
                }
                for k in 0..per_sheet {
                    // A cell's pile gets consecutive pages. Missing pages leave blank cells,
                    // rather than shifting later piles left on the final sheets.
                    let index = if order == PageOrder::CutStack {
                        k.checked_mul(sheet_count)
                            .and_then(|n| n.checked_add(sheet_index))
                    } else {
                        sheet_index
                            .checked_mul(per_sheet)
                            .and_then(|n| n.checked_add(k))
                    };
                    let Some(p) = index.and_then(|i| pages.get(i)).copied() else {
                        continue;
                    };

                    let (col, row) = match order {
                        PageOrder::Horizontal | PageOrder::CutStack => (k % gc, k / gc),
                        PageOrder::HorizontalReversed => (gc - 1 - k % gc, k / gc),
                        PageOrder::Vertical => (k / gr, k % gr),
                        PageOrder::VerticalReversed => (gc - 1 - k / gr, k % gr),
                    };
                    let x0 = MARGIN + col as f64 * (cw + gap);
                    let y1 = size.1 - MARGIN - row as f64 * (ch + gap);
                    let cell = [x0, y1 - ch, x0 + cw, y1];
                    let d = sizes
                        .get(p)
                        .copied()
                        .ok_or_else(|| LayoutError::Invalid("Sayfa boyutu eksik".into()))?;
                    if !(d.0.is_finite() && d.1.is_finite() && d.0 > 0.0 && d.1 > 0.0) {
                        return Err(LayoutError::Invalid("Sayfa boyutu geçersiz".into()));
                    }
                    let rotate = auto_rotate && (d.0 > d.1) != (cw > ch) && (d.0 - d.1).abs() > 1.0;
                    let s = fit_scale(if rotate { (d.1, d.0) } else { d }, (cw, ch));
                    if !s.is_finite() || s <= 0.0 {
                        return Err(LayoutError::Invalid("Sayfa ızgaraya ölçeklenemiyor".into()));
                    }
                    let pl = place_in(p, d, cell, s, rotate);
                    if border {
                        let b = pl.matrix.bbox(pl.clip);
                        sheet.borders.push(b);
                    }
                    sheet.placed.push(pl);
                }
                sheets.push(sheet);
            }
        }
        Layout::Booklet { subset, binding } => {
            // Pad to a multiple of 4 with blank pages (None).
            let n4 = pages.len().div_ceil(4) * 4;
            let at = |i: usize| pages.get(i).copied();
            let size = oriented(paper, Orientation::Landscape, true);
            let half = (size.0 - 2.0 * MARGIN) / 2.0;
            let cells = [
                [MARGIN, MARGIN, MARGIN + half, size.1 - MARGIN],
                [MARGIN + half, MARGIN, size.0 - MARGIN, size.1 - MARGIN],
            ];
            for k in 0..n4 / 4 {
                let front = (at(n4 - 1 - 2 * k), at(2 * k));
                let back = (at(2 * k + 1), at(n4 - 2 - 2 * k));
                let mut sides = Vec::new();
                if subset != BookletSubset::BackOnly {
                    sides.push(front);
                }
                if subset != BookletSubset::FrontOnly {
                    sides.push(back);
                }
                for (l, r) in sides {
                    let (l, r) = if binding == Binding::Right {
                        (r, l)
                    } else {
                        (l, r)
                    };
                    let mut sheet = Sheet {
                        size,
                        ..Sheet::default()
                    };
                    for (p, cell) in [(l, cells[0]), (r, cells[1])] {
                        if let Some(p) = p {
                            let d = sizes[p];
                            let s = fit_scale(d, (cell[2] - cell[0], cell[3] - cell[1]));
                            sheet.placed.push(place_in(p, d, cell, s, false));
                        }
                    }
                    sheets.push(sheet);
                }
            }
        }
        Layout::Poster {
            scale,
            overlap,
            cut_marks,
        } => {
            if !(scale.is_finite() && scale > 0.0 && overlap.is_finite() && overlap >= 0.0) {
                return Err(LayoutError::Invalid(
                    "Poster ölçeği ya da bindirme payı geçersiz".into(),
                ));
            }
            let s = scale / 100.0;
            for &p in &pages {
                let d = sizes[p];
                let size = oriented(paper, settings.orientation, d.0 > d.1);
                // Each tile shows `step` of the scaled page plus `overlap` shared with the next.
                let (tw, th) = (size.0 - 2.0 * MARGIN, size.1 - 2.0 * MARGIN);
                if overlap * 2.0 >= tw.min(th) {
                    return Err(LayoutError::Invalid("Bindirme payı tabakadan büyük".into()));
                }
                let (sw, sh) = (d.0 * s, d.1 * s);
                let (stepx, stepy) = (tw - overlap, th - overlap);
                let nx = (((sw - overlap) / stepx).ceil() as usize).max(1);
                let ny = (((sh - overlap) / stepy).ceil() as usize).max(1);
                // The counts saturate at usize::MAX for a huge scale, so their product can overflow.
                if nx.checked_mul(ny).is_none_or(|n| n > 1024) {
                    return Err(LayoutError::Invalid(
                        "Poster 1024'ten fazla tabaka gerektiriyor".into(),
                    ));
                }
                // Tiles from the top-left, row by row.
                for ty in 0..ny {
                    for tx in 0..nx {
                        // The window into the scaled page (scaled display space, y up).
                        let wx0 = tx as f64 * stepx;
                        let wy1 = sh - ty as f64 * stepy;
                        let matrix = Matrix([s, 0.0, 0.0, s, MARGIN - wx0, size.1 - MARGIN - wy1]);
                        let clip = [wx0 / s, (wy1 - th) / s, (wx0 + tw) / s, wy1 / s];
                        let mut sheet = Sheet {
                            size,
                            placed: vec![Placement {
                                page: p,
                                matrix,
                                clip,
                            }],
                            ..Sheet::default()
                        };
                        if cut_marks {
                            let (x0, y0, x1, y1) =
                                (MARGIN, MARGIN, size.0 - MARGIN, size.1 - MARGIN);
                            for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
                                let dx = if x == x0 { -1.0 } else { 1.0 };
                                let dy = if y == y0 { -1.0 } else { 1.0 };
                                sheet
                                    .lines
                                    .push([x + dx * 2.0, y, x + dx * (MARGIN - 2.0), y]);
                                sheet
                                    .lines
                                    .push([x, y + dy * 2.0, x, y + dy * (MARGIN - 2.0)]);
                            }
                        }
                        sheets.push(sheet);
                    }
                }
            }
        }
    }
    Ok(sheets)
}

/// Which source pages land on each sheet (for previews and the sheet count).
pub fn sheet_pages(sheets: &[Sheet]) -> Vec<Vec<usize>> {
    sheets
        .iter()
        .map(|s| s.placed.iter().map(|p| p.page).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(pages: Vec<usize>, layout: Layout) -> Settings {
        Settings {
            pages,
            paper: (612.0, 792.0),
            layout,
            ..Settings::default()
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.01
    }

    fn cut_stack(cols: usize, rows: usize) -> Layout {
        Layout::Multiple {
            cols,
            rows,
            per_sheet: cols.saturating_mul(rows),
            order: PageOrder::CutStack,
            border: false,
            auto_rotate: false,
        }
    }

    #[test]
    fn size_modes() {
        let sizes = [(200.0, 300.0), (1000.0, 1500.0)];
        let fit = layout(&sizes, &settings(vec![0, 1], Layout::Size(SizeMode::Fit))).unwrap();
        let s0 = fit[0].placed[0].matrix.0[0];
        assert!(close(s0, 2.52), "min(576 / 200, 756 / 300): {s0}");
        let actual = layout(&sizes, &settings(vec![0], Layout::Size(SizeMode::Actual))).unwrap();
        assert_eq!(
            actual[0].placed[0].matrix.0,
            [1.0, 0.0, 0.0, 1.0, 206.0, 246.0],
            "centred at 100%"
        );
        let shrink = layout(
            &sizes,
            &settings(vec![0, 1], Layout::Size(SizeMode::Shrink)),
        )
        .unwrap();
        assert_eq!(
            shrink[0].placed[0].matrix.0[0], 1.0,
            "small pages stay at 100%"
        );
        assert!(shrink[1].placed[0].matrix.0[0] < 1.0, "big pages shrink");
        let custom = layout(
            &sizes,
            &settings(vec![0], Layout::Size(SizeMode::Custom(50.0))),
        )
        .unwrap();
        assert_eq!(custom[0].placed[0].matrix.0[0], 0.5);
        // Auto orientation turns the sheet for landscape pages.
        let land = layout(
            &[(300.0, 200.0)],
            &settings(vec![0], Layout::Size(SizeMode::Fit)),
        )
        .unwrap();
        assert_eq!(land[0].size, (792.0, 612.0));
    }

    #[test]
    fn multiple_pages_per_sheet() {
        let sizes = vec![(200.0, 300.0); 5];
        let sheets = layout(&sizes, &settings((0..5).collect(), Layout::multiple(4))).unwrap();
        assert_eq!(sheet_pages(&sheets), [vec![0, 1, 2, 3], vec![4]]);
        // Horizontal order: 0 top-left, 1 top-right, 2 bottom-left.
        let o = |k: usize| {
            (
                sheets[0].placed[k].matrix.0[4],
                sheets[0].placed[k].matrix.0[5],
            )
        };
        assert!(
            o(1).0 > o(0).0 && close(o(1).1, o(0).1) && o(2).1 < o(0).1 && close(o(2).0, o(0).0)
        );
        // Two per sheet prints side by side on landscape paper.
        let two = layout(&sizes, &settings(vec![0, 1], Layout::multiple(2))).unwrap();
        assert_eq!(two[0].size, (792.0, 612.0));
        assert!(two[0].placed[1].matrix.0[4] > two[0].placed[0].matrix.0[4]);
        // Vertical order and borders.
        let v = layout(
            &sizes,
            &settings(
                vec![0, 1, 2],
                Layout::Multiple {
                    cols: 2,
                    rows: 2,
                    per_sheet: 4,
                    order: PageOrder::Vertical,
                    border: true,
                    auto_rotate: false,
                },
            ),
        )
        .unwrap();
        assert!(
            close(v[0].placed[1].matrix.0[4], v[0].placed[0].matrix.0[4])
                && v[0].placed[1].matrix.0[5] < v[0].placed[0].matrix.0[5]
        );
        assert_eq!(v[0].borders.len(), 3);
        // Auto-rotate turns a landscape page in a portrait cell.
        let r = layout(
            &[(300.0, 200.0), (200.0, 300.0)],
            &settings(vec![0, 1], Layout::multiple(4)),
        )
        .unwrap();
        // The first (landscape) page sets a landscape sheet; the portrait page is turned to fit.
        assert_eq!(r[0].size, (792.0, 612.0));
        assert!(r[0].placed[0].matrix.0[0] > 0.0);
        assert_eq!(r[0].placed[1].matrix.0[0], 0.0, "rotated");
    }

    #[test]
    fn custom_pages_per_sheet_never_overfill_the_grid() {
        // Three per sheet draws on a 2 × 2 grid, five on 3 × 2: the spare cells stay blank (#1336).
        let sizes = vec![(200.0, 300.0); 7];
        let pages = |per: usize| {
            sheet_pages(
                &layout(&sizes, &settings((0..7).collect(), Layout::multiple(per))).unwrap(),
            )
        };
        assert_eq!(pages(3), [vec![0, 1, 2], vec![3, 4, 5], vec![6]]);
        assert_eq!(pages(5), [vec![0, 1, 2, 3, 4], vec![5, 6]]);
        assert_eq!(pages(7), [vec![0, 1, 2, 3, 4, 5, 6]]);
        assert_eq!(pages(4), [vec![0, 1, 2, 3], vec![4, 5, 6]]);
        assert_eq!(pages(2), [vec![0, 1], vec![2, 3], vec![4, 5], vec![6]]);
        // Cut-stack piles take the requested count too.
        let stacked = Layout::Multiple {
            cols: 2,
            rows: 2,
            per_sheet: 3,
            order: PageOrder::CutStack,
            border: false,
            auto_rotate: false,
        };
        assert_eq!(
            sheet_pages(&layout(&sizes, &settings((0..7).collect(), stacked)).unwrap()),
            [vec![0, 3, 6], vec![1, 4], vec![2, 5]]
        );
        // A count larger than the grid is capped by the grid; no pages per sheet is refused.
        let grid = |per_sheet| Layout::Multiple {
            cols: 2,
            rows: 1,
            per_sheet,
            order: PageOrder::Horizontal,
            border: false,
            auto_rotate: false,
        };
        assert_eq!(
            sheet_pages(&layout(&sizes[..3], &settings(vec![0, 1, 2], grid(9))).unwrap()),
            [vec![0, 1], vec![2]]
        );
        assert!(matches!(
            layout(&sizes, &settings(vec![0], grid(0))),
            Err(LayoutError::Invalid(_))
        ));
    }

    #[test]
    fn booklets_pair_pages_for_folding() {
        let sizes = vec![(200.0, 300.0); 6];
        let sheets = layout(
            &sizes,
            &settings(
                (0..6).collect(),
                Layout::Booklet {
                    subset: BookletSubset::BothSides,
                    binding: Binding::Left,
                },
            ),
        )
        .unwrap();
        // 6 pages pad to 8: sheet 1 front [8,1] → [blank, 0], back [1, 6]; sheet 2 front [5, 2], back [3, 4].
        assert_eq!(
            sheet_pages(&sheets),
            [vec![0], vec![1], vec![5, 2], vec![3, 4]]
        );
        assert!(
            sheets[0].placed[0].matrix.0[4] >= 395.9,
            "page 1 on the right half"
        );
        assert_eq!(sheets[0].size, (792.0, 612.0));
        let front = layout(
            &sizes,
            &settings(
                (0..6).collect(),
                Layout::Booklet {
                    subset: BookletSubset::FrontOnly,
                    binding: Binding::Right,
                },
            ),
        )
        .unwrap();
        assert_eq!(
            sheet_pages(&front),
            [vec![0], vec![2, 5]],
            "right binding mirrors the spread"
        );
    }

    #[test]
    fn posters_tile_with_overlap() {
        // A 200×300 page at 400% = 800×1200 on Letter with 18 pt margins (576×756 tiles), overlap 36.
        let sheets = layout(
            &[(200.0, 300.0)],
            &settings(
                vec![0],
                Layout::Poster {
                    scale: 400.0,
                    overlap: 36.0,
                    cut_marks: true,
                },
            ),
        )
        .unwrap();
        assert_eq!(sheets.len(), 4, "2 × 2 tiles");
        assert_eq!(sheets[0].lines.len(), 8, "cut marks");
        let c0 = sheets[0].placed[0].clip;
        let c1 = sheets[1].placed[0].clip;
        assert!(
            close(c0[2] - c1[0], 36.0 / 4.0),
            "neighbouring tiles share the overlap"
        );
        assert!(layout(
            &[(200.0, 300.0)],
            &settings(
                vec![0],
                Layout::Poster {
                    scale: 400.0,
                    overlap: 400.0,
                    cut_marks: false
                }
            )
        )
        .is_err());
    }

    /// Issue #1335: a scale so large that the tile counts saturate is refused with the sheet-limit
    /// error, not an overflow in counting the sheets.
    #[test]
    fn huge_posters_are_refused_at_the_sheet_limit() {
        let refused = Err(LayoutError::Invalid(
            "Poster 1024'ten fazla tabaka gerektiriyor".into(),
        ));
        for scale in [100_000.0, 1e22, f64::MAX] {
            let poster = settings(
                vec![0],
                Layout::Poster {
                    scale,
                    overlap: 18.0,
                    cut_marks: true,
                },
            );
            assert_eq!(layout(&[(200.0, 300.0)], &poster), refused, "scale {scale}");
        }
    }

    #[test]
    fn cut_stack_keeps_piles_in_order_and_blanks_in_place() {
        let sizes = vec![(200.0, 300.0); 10];
        let sheets = layout(&sizes, &settings((0..10).collect(), cut_stack(2, 2))).unwrap();
        assert_eq!(
            sheet_pages(&sheets),
            [vec![0, 3, 6, 9], vec![1, 4, 7], vec![2, 5, 8]]
        );
        for s in &sheets {
            assert_eq!(
                s.lines, sheets[0].lines,
                "identical cuts even with blank cells"
            );
            assert_eq!(s.lines.len(), 4);
            for (pl, first) in s.placed.iter().zip(&sheets[0].placed) {
                assert_eq!(pl.matrix, first.matrix, "every pile stays in its cell");
            }
        }
        assert!(sheets[0].placed[1].matrix.0[4] > sheets[0].placed[0].matrix.0[4]);
        assert!(sheets[0].placed[2].matrix.0[5] < sheets[0].placed[0].matrix.0[5]);
        // Selection and reverse order are preserved, including repeated source pages.
        let selected = vec![9, 5, 5, 2, 0];
        let sheets = layout(&sizes, &settings(selected, cut_stack(2, 2))).unwrap();
        assert_eq!(sheet_pages(&sheets), [vec![9, 5, 0], vec![5, 2]]);
        assert_eq!(sheets[1].placed[1].matrix, sheets[0].placed[1].matrix);
    }

    #[test]
    fn cutting_and_stacking_recovers_every_selected_page() {
        // Simulate the actual operation using cell positions, not the imposition formula.
        for (cols, rows) in [(1, 1), (1, 2), (2, 2), (2, 3), (3, 3), (4, 4)] {
            for orientation in [
                Orientation::Auto,
                Orientation::Portrait,
                Orientation::Landscape,
            ] {
                for n in 1..=37 {
                    let sizes = vec![(200.0, 300.0); n];
                    let selected: Vec<usize> = (0..n).rev().collect();
                    let s = Settings {
                        orientation,
                        ..settings(selected.clone(), cut_stack(cols, rows))
                    };
                    let sheets = layout(&sizes, &s).unwrap();
                    let mut piles: std::collections::BTreeMap<(i64, i64), Vec<usize>> =
                        Default::default();
                    for sheet in &sheets {
                        for pl in &sheet.placed {
                            // Equal source sizes, so the page origins identify row/column.
                            let [_, _, _, _, x, y] = pl.matrix.0;
                            piles
                                .entry((-(y * 100.0).round() as i64, (x * 100.0).round() as i64))
                                .or_default()
                                .push(pl.page);
                        }
                    }
                    let restacked: Vec<usize> = piles.into_values().flatten().collect();
                    assert_eq!(
                        restacked, selected,
                        "{cols}x{rows}, {n} pages, {orientation:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn multiple_rejects_hostile_grids_without_panicking() {
        let sizes = [(200.0, 300.0)];
        for (cols, rows) in [(usize::MAX, 2), (2, usize::MAX), (0, 2), (1, 0), (257, 1)] {
            let mode = Layout::Multiple {
                cols,
                rows,
                per_sheet: 1,
                order: PageOrder::Horizontal,
                border: false,
                auto_rotate: false,
            };
            assert!(matches!(
                layout(&sizes, &settings(vec![0], mode)),
                Err(LayoutError::Invalid(_))
            ));
        }
        let tiny = Settings {
            paper: (72.0, 72.0),
            ..settings(vec![0], cut_stack(16, 16))
        };
        assert!(matches!(
            layout(&sizes, &tiny),
            Err(LayoutError::Invalid(_))
        ));
        for size in [
            (0.0, 300.0),
            (200.0, f64::NAN),
            (f64::INFINITY, 300.0),
            (f64::from_bits(1), f64::from_bits(1)),
        ] {
            assert!(matches!(
                layout(&[size], &settings(vec![0], cut_stack(2, 2))),
                Err(LayoutError::Invalid(_))
            ));
        }
    }

    #[test]
    fn architectural_sheets_preserve_full_scale() {
        let index = matching_paper((2592.0, 1728.0)).unwrap();
        assert_eq!(PAPERS[index].0, "ARCH D (24 x 36 in)");
        assert_eq!(matching_paper((1728.0, 2592.0)), Some(index));
        assert_eq!(matching_paper((f64::NAN, 2592.0)), None);
        let s = Settings {
            pages: vec![0],
            paper: PAPERS[index].1,
            layout: Layout::Size(SizeMode::Actual),
            ..Settings::default()
        };
        let sheets = layout(&[(2592.0, 1728.0)], &s).unwrap();
        assert_eq!(sheets[0].size, (2592.0, 1728.0));
        assert_eq!(sheets[0].placed[0].matrix.0, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let fit = layout(
            &[(2592.0, 1728.0)],
            &Settings {
                layout: Layout::Size(SizeMode::Fit),
                ..s
            },
        )
        .unwrap();
        assert!(
            fit[0].placed[0].matrix.0[0] < 1.0,
            "Fit adds margins and must not masquerade as full scale"
        );
    }

    /// A sheet matches a preset either way round and rounded to whole points, but not half a point off;
    /// empty, negative, infinite, NaN and custom sizes match none.
    #[test]
    fn paper_matching_accepts_rounded_metric_sizes_but_not_custom_or_invalid_sizes() {
        for (i, (_, (w, h))) in PAPERS.iter().enumerate() {
            assert_eq!(matching_paper((*w, *h)), Some(i));
            assert_eq!(matching_paper((*h, *w)), Some(i));
            assert_eq!(matching_paper((w.round(), h.round())), Some(i));
            assert_eq!(matching_paper((w + 0.51, *h)), None);
        }
        for size in [
            (0.0, 0.0),
            (-1728.0, 2592.0),
            (f64::INFINITY, 2592.0),
            (1728.0, f64::NAN),
            (1000.0, 1000.0),
        ] {
            assert_eq!(matching_paper(size), None);
        }
    }

    /// #2734: the US #10 envelope (4.125 × 9.5 in) is offered, fed by its PPD name and laid out at
    /// its own size.
    #[test]
    fn us_number_10_envelope_is_offered_and_fed_by_name() {
        let index = matching_paper((684.0, 297.0)).unwrap();
        assert_eq!(PAPERS[index].0, "US #10 Envelope (4.125 x 9.5 in)");
        assert_eq!(PAPERS[index].1, (297.0, 684.0));
        assert_eq!(cups_media(index), Some("Env10"));
        let s = Settings {
            pages: vec![0],
            paper: PAPERS[index].1,
            layout: Layout::Size(SizeMode::Actual),
            ..Settings::default()
        };
        assert_eq!(
            layout(&[(297.0, 684.0)], &s).unwrap()[0].size,
            (297.0, 684.0)
        );
    }
}
