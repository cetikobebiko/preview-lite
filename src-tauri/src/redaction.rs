use crate::document_session::{SessionJobOperationError, SessionJobOutcome};
use pdfium_render::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub use crate::secure_redaction;
pub use crate::secure_redaction::*;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RedactionArea {
    pub page_index: u32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SanitizationOptions {
    pub remove_metadata: bool,
    pub remove_annotations: bool,
    pub remove_javascript: bool,
    pub remove_embedded_files: bool,
    pub remove_hidden_layers: bool, // OCG
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GeometryStatus {
    Resolved,
    Unresolved,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoordinateSpace {
    PdfUserSpace,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GeometryUnit {
    PdfPoint,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoxOrigin {
    BottomLeft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntrinsicRotation {
    Deg0 = 0,
    Deg90 = 90,
    Deg180 = 180,
    Deg270 = 270,
}

impl IntrinsicRotation {
    pub fn as_u16(self) -> u16 {
        self as u16
    }
}

impl TryFrom<u16> for IntrinsicRotation {
    type Error = String;
    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Deg0),
            90 => Ok(Self::Deg90),
            180 => Ok(Self::Deg180),
            270 => Ok(Self::Deg270),
            other => Err(format!("Invalid intrinsic_rotation: {other}")),
        }
    }
}

impl Serialize for IntrinsicRotation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u16(self.as_u16())
    }
}

impl<'de> Deserialize<'de> for IntrinsicRotation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = u16::deserialize(deserializer)?;
        IntrinsicRotation::try_from(raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PdfPageBox {
    pub left: f32,
    pub bottom: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PageGeometry {
    pub coordinate_space: CoordinateSpace,
    pub unit: GeometryUnit,
    pub origin: BoxOrigin,
    pub media_box: PdfPageBox,
    pub crop_box: Option<PdfPageBox>,
    pub effective_box: PdfPageBox,
    pub intrinsic_rotation: IntrinsicRotation,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PdfRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScanWarning {
    pub code: String,
    pub message: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SensitiveScanItem {
    pub page_index: u32,
    pub pattern_index: u32,
    pub text_preview: String,
    pub geometry_status: GeometryStatus,
    pub page_geometry: Option<PageGeometry>,
    pub source_rects_pdf_points: Vec<PdfRect>,
    pub warnings: Vec<ScanWarning>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SensitiveScanReport {
    pub schema_version: u32,
    pub items: Vec<SensitiveScanItem>,
    pub warnings: Vec<ScanWarning>,
    pub truncated: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ScanItemPushResult {
    Added,
    LimitExceeded,
}

pub(crate) fn push_sensitive_scan_item(
    report: &mut SensitiveScanReport,
    item: SensitiveScanItem,
) -> ScanItemPushResult {
    if report.items.len() >= 10_000 {
        report.truncated = true;
        push_report_warning(
            &mut report.warnings,
            "SENSITIVE_SCAN_ITEM_LIMIT_EXCEEDED",
            "Sonuç sınırı (10000) aşıldı, tarama durduruldu.",
        );
        ScanItemPushResult::LimitExceeded
    } else {
        report.items.push(item);
        ScanItemPushResult::Added
    }
}

pub(crate) fn calculate_scan_page_bounds(total_pages: usize, max_pages: usize) -> (usize, bool) {
    if total_pages > max_pages {
        (max_pages, true)
    } else {
        (total_pages, false)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CanonicalCharacter {
    pub(crate) value: char,
    pub(crate) source_character_index: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct CanonicalRun {
    pub(crate) text: String,
    pub(crate) characters: Vec<CanonicalCharacter>,
    pub(crate) byte_boundaries: Vec<usize>,
}

impl CanonicalRun {
    pub(crate) fn new() -> Self {
        Self {
            text: String::new(),
            characters: Vec::new(),
            byte_boundaries: vec![0],
        }
    }

    pub(crate) fn push(&mut self, ch: char, source_idx: usize) {
        self.text.push(ch);
        self.characters.push(CanonicalCharacter {
            value: ch,
            source_character_index: source_idx,
        });
        self.byte_boundaries.push(self.text.len());
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

pub(crate) struct PureSourceChar {
    pub(crate) source_index: usize,
    pub(crate) unicode: Option<char>,
    pub(crate) bounds: Option<(f32, f32, f32, f32)>,
}

pub(crate) struct ExtractedPageData {
    pub(crate) runs: Vec<CanonicalRun>,
    pub(crate) char_bounds: Vec<Option<(f32, f32, f32, f32)>>,
}

pub(crate) fn validate_patterns(
    patterns: &[String],
) -> Result<Vec<Regex>, SessionJobOperationError> {
    if patterns.len() > 64 {
        return Err(SessionJobOperationError::failed_with_code(
            "SENSITIVE_SCAN_PATTERN_LIMIT_EXCEEDED",
            "En fazla 64 pattern gönderilebilir",
        ));
    }

    let mut total_bytes = 0usize;
    for pattern in patterns {
        let len = pattern.len();
        if len > 1024 {
            return Err(SessionJobOperationError::failed_with_code(
                "SENSITIVE_SCAN_PATTERN_LIMIT_EXCEEDED",
                "Pattern boyutu 1024 UTF-8 baytını aşamaz",
            ));
        }
        total_bytes += len;
    }

    if total_bytes > 32 * 1024 {
        return Err(SessionJobOperationError::failed_with_code(
            "SENSITIVE_SCAN_PATTERN_LIMIT_EXCEEDED",
            "Toplam pattern boyutu 32 KiB sınırını aşamaz",
        ));
    }

    let mut compiled = Vec::with_capacity(patterns.len());
    for (index, pattern) in patterns.iter().enumerate() {
        match Regex::new(pattern) {
            Ok(re) => compiled.push(re),
            Err(_) => {
                return Err(SessionJobOperationError::failed_with_code(
                    "SENSITIVE_SCAN_INVALID_PATTERN",
                    format!("Pattern index {index} geçersiz regex sözdizimi içeriyor."),
                ));
            }
        }
    }

    Ok(compiled)
}

pub(crate) fn build_canonical_runs_pure<I>(
    chars: I,
    current_char_count: &mut usize,
    max_text_chars: usize,
) -> (ExtractedPageData, bool)
where
    I: Iterator<Item = PureSourceChar>,
{
    let mut runs = Vec::new();
    let mut current_run = CanonicalRun::new();
    let mut char_bounds = Vec::new();
    let mut hit_text_limit = false;

    for source_char in chars {
        if *current_char_count >= max_text_chars {
            hit_text_limit = true;
            break;
        }

        *current_char_count += 1;
        char_bounds.push(source_char.bounds);

        if let Some(ch) = source_char.unicode {
            current_run.push(ch, source_char.source_index);
        } else {
            if !current_run.is_empty() {
                runs.push(current_run);
                current_run = CanonicalRun::new();
            }
        }
    }

    if !current_run.is_empty() {
        runs.push(current_run);
    }

    (ExtractedPageData { runs, char_bounds }, hit_text_limit)
}

pub(crate) fn compute_segments_from_bounds_pure<F>(
    matched_can_chars: &[CanonicalCharacter],
    get_bounds: F,
    page_width: f32,
    page_height: f32,
) -> (GeometryStatus, Vec<PdfRect>, usize)
where
    F: Fn(usize) -> Option<(f32, f32, f32, f32)>,
{
    if !page_width.is_finite()
        || !page_height.is_finite()
        || page_width <= 0.0
        || page_height <= 0.0
    {
        return (GeometryStatus::Unresolved, Vec::new(), 0);
    }

    let mut line_rects: Vec<PdfRect> = Vec::new();
    let mut current_union: Option<(f32, f32, f32, f32)> = None;
    let mut prev_valid_bounds: Option<(f32, f32, f32, f32)> = None;

    for can_char in matched_can_chars {
        let bounds_opt = get_bounds(can_char.source_character_index);
        let Some((min_x, min_y, max_x, max_y)) = bounds_opt else {
            if !can_char.value.is_whitespace() {
                return (GeometryStatus::Unresolved, Vec::new(), 0);
            }
            continue;
        };

        if ![min_x, max_x, min_y, max_y].iter().all(|v| v.is_finite())
            || max_x <= min_x
            || max_y <= min_y
        {
            if !can_char.value.is_whitespace() {
                return (GeometryStatus::Unresolved, Vec::new(), 0);
            }
            continue;
        }

        match (current_union, prev_valid_bounds) {
            (None, None) => {
                current_union = Some((min_x, min_y, max_x, max_y));
                prev_valid_bounds = Some((min_x, min_y, max_x, max_y));
            }
            (
                Some((u_min_x, u_min_y, u_max_x, u_max_y)),
                Some((_p_min_x, p_min_y, _p_max_x, p_max_y)),
            ) => {
                let overlap_min = p_min_y.max(min_y);
                let overlap_max = p_max_y.min(max_y);
                let overlap_h = (overlap_max - overlap_min).max(0.0);

                let h_prev = p_max_y - p_min_y;
                let h_char = max_y - min_y;
                let min_h = h_prev.min(h_char);

                if min_h > 0.0 && overlap_h >= 0.5 * min_h {
                    current_union = Some((
                        u_min_x.min(min_x),
                        u_min_y.min(min_y),
                        u_max_x.max(max_x),
                        u_max_y.max(max_y),
                    ));
                    prev_valid_bounds = Some((min_x, min_y, max_x, max_y));
                } else {
                    line_rects.push(PdfRect {
                        x: u_min_x,
                        y: u_min_y,
                        width: u_max_x - u_min_x,
                        height: u_max_y - u_min_y,
                    });
                    current_union = Some((min_x, min_y, max_x, max_y));
                    prev_valid_bounds = Some((min_x, min_y, max_x, max_y));
                }
            }
            _ => unreachable!(),
        }
    }

    if let Some((u_min_x, u_min_y, u_max_x, u_max_y)) = current_union {
        line_rects.push(PdfRect {
            x: u_min_x,
            y: u_min_y,
            width: u_max_x - u_min_x,
            height: u_max_y - u_min_y,
        });
    }

    let total_segment_count = line_rects.len();
    if total_segment_count == 0 || total_segment_count > 64 {
        (GeometryStatus::Unresolved, Vec::new(), total_segment_count)
    } else {
        (GeometryStatus::Resolved, line_rects, total_segment_count)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FpdfFsRectf {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

pub(crate) fn extract_page_geometry(pdfium: &Pdfium, page: &PdfPage) -> Option<PageGeometry> {
    let media = page.boundaries().media().ok()?;
    let media_left = media.bounds.left().value;
    let media_bottom = media.bounds.bottom().value;
    let media_right = media.bounds.right().value;
    let media_top = media.bounds.top().value;
    let media_width = media_right - media_left;
    let media_height = media_top - media_bottom;
    if !media_left.is_finite()
        || !media_bottom.is_finite()
        || media_width <= 0.0
        || media_height <= 0.0
    {
        return None;
    }
    let media_box = PdfPageBox {
        left: media_left,
        bottom: media_bottom,
        width: media_width,
        height: media_height,
    };

    let crop_box = if let Ok(crop) = page.boundaries().crop() {
        let crop_left = crop.bounds.left().value;
        let crop_bottom = crop.bounds.bottom().value;
        let crop_right = crop.bounds.right().value;
        let crop_top = crop.bounds.top().value;
        let crop_width = crop_right - crop_left;
        let crop_height = crop_top - crop_bottom;
        if crop_left.is_finite() && crop_bottom.is_finite() && crop_width > 0.0 && crop_height > 0.0
        {
            Some(PdfPageBox {
                left: crop_left,
                bottom: crop_bottom,
                width: crop_width,
                height: crop_height,
            })
        } else {
            return None;
        }
    } else {
        None
    };

    let bindings = pdfium.bindings();
    let page_handle = bindings.get_handle_from_page(page);
    let mut fs_rect = FpdfFsRectf::default();
    let success =
        bindings.FPDF_GetPageBoundingBox(page_handle, &mut fs_rect as *mut FpdfFsRectf as *mut _);
    if success == 0 {
        return None;
    }
    let eff_left = fs_rect.left;
    let eff_bottom = fs_rect.bottom;
    let eff_width = fs_rect.right - fs_rect.left;
    let eff_height = fs_rect.top - fs_rect.bottom;
    if !eff_left.is_finite() || !eff_bottom.is_finite() || eff_width <= 0.0 || eff_height <= 0.0 {
        return None;
    }
    let effective_box = PdfPageBox {
        left: eff_left,
        bottom: eff_bottom,
        width: eff_width,
        height: eff_height,
    };

    let rot = page.rotation().ok()?;
    let deg = (rot.as_degrees() as i32).rem_euclid(360);
    let intrinsic_rotation = match deg {
        0 => IntrinsicRotation::Deg0,
        90 => IntrinsicRotation::Deg90,
        180 => IntrinsicRotation::Deg180,
        270 => IntrinsicRotation::Deg270,
        _ => return None,
    };

    Some(PageGeometry {
        coordinate_space: CoordinateSpace::PdfUserSpace,
        unit: GeometryUnit::PdfPoint,
        origin: BoxOrigin::BottomLeft,
        media_box,
        crop_box,
        effective_box,
        intrinsic_rotation,
    })
}

pub(crate) fn resolve_item_geometry_and_rects(
    calc_status: GeometryStatus,
    page_geometry: Option<PageGeometry>,
    rects: Vec<PdfRect>,
    item_warnings: &mut Vec<ScanWarning>,
) -> (GeometryStatus, Option<PageGeometry>, Vec<PdfRect>) {
    let (final_status, item_geom) = match (calc_status, page_geometry) {
        (GeometryStatus::Resolved, Some(geom)) => (GeometryStatus::Resolved, Some(geom)),
        (_, Some(geom)) => (GeometryStatus::Unresolved, Some(geom)),
        (_, None) => {
            item_warnings.push(ScanWarning {
                code: "SENSITIVE_SCAN_PAGE_GEOMETRY_UNRESOLVED".into(),
                message: "Sayfa geometrisi çıkarılamadı.".into(),
            });
            (GeometryStatus::Unresolved, None)
        }
    };

    let final_rects = if final_status == GeometryStatus::Resolved {
        rects
    } else {
        Vec::new()
    };

    (final_status, item_geom, final_rects)
}

pub(crate) fn scan_sensitive_file(
    snapshot_path: &std::path::Path,
    credential: Option<&str>,
    patterns: &[String],
) -> Result<SensitiveScanReport, SessionJobOperationError> {
    let compiled_regexes = validate_patterns(patterns)?;

    crate::pdf_engine::with_pdfium(|pdfium| {
        let document = pdfium
            .load_pdf_from_file(snapshot_path, credential)
            .map_err(|_| {
                SessionJobOperationError::failed_with_code(
                    "SESSION_JOB_PDF_OPEN_FAILED",
                    "Sensitive-data scan PDF snapshot açılamadı",
                )
            })?;

        let mut report = SensitiveScanReport {
            schema_version: 2,
            items: Vec::new(),
            warnings: Vec::new(),
            truncated: false,
        };

        let page_count = usize::from(document.pages().len());
        let (pages_to_scan, page_limit_hit) =
            calculate_scan_page_bounds(page_count, crate::job_worker::MAX_JOB_PAGES);
        if page_limit_hit {
            report.truncated = true;
            push_report_warning(
                &mut report.warnings,
                "SENSITIVE_SCAN_PAGE_LIMIT_EXCEEDED",
                "Sayfa sınırı aşıldı, tarama durduruldu.",
            );
        }

        let mut current_total_char_count = 0usize;
        let mut has_segment_limit_warning = false;

        'page_loop: for page_index in 0..pages_to_scan {
            let Ok(page) = document.pages().get(page_index as u16) else {
                continue;
            };
            let page_geometry = extract_page_geometry(pdfium, &page);

            let Ok(text_page) = page.text() else {
                continue;
            };

            let page_width = page.width().value;
            let page_height = page.height().value;

            let page_chars = text_page.chars();
            let (extracted_data, hit_text_limit) = build_canonical_runs_pure(
                page_chars
                    .iter()
                    .enumerate()
                    .map(|(idx, c)| PureSourceChar {
                        source_index: idx,
                        unicode: c.unicode_char(),
                        bounds: c.loose_bounds().ok().and_then(|b| {
                            let min_x = b.left().value.min(b.right().value);
                            let max_x = b.left().value.max(b.right().value);
                            let min_y = b.bottom().value.min(b.top().value);
                            let max_y = b.top().value.max(b.bottom().value);
                            if [min_x, max_x, min_y, max_y].iter().all(|v| v.is_finite())
                                && max_x > min_x
                                && max_y > min_y
                            {
                                Some((min_x, min_y, max_x, max_y))
                            } else {
                                None
                            }
                        }),
                    }),
                &mut current_total_char_count,
                crate::job_worker::MAX_JOB_TEXT_CHARS,
            );

            if hit_text_limit {
                report.truncated = true;
                push_report_warning(
                    &mut report.warnings,
                    "SENSITIVE_SCAN_TEXT_LIMIT_EXCEEDED",
                    "Metin karakter sınırı aşıldı, tarama durduruldu.",
                );
            }

            for (pattern_index, regex) in compiled_regexes.iter().enumerate() {
                for run in &extracted_data.runs {
                    for matched in regex.find_iter(&run.text) {
                        let start_res = run.byte_boundaries.binary_search(&matched.start());
                        let end_res = run.byte_boundaries.binary_search(&matched.end());

                        let matched_can_chars = match (start_res, end_res) {
                            (Ok(s), Ok(e)) if s <= e => &run.characters[s..e],
                            _ => &[],
                        };

                        let (calc_status, rects, total_segment_count) =
                            compute_segments_from_bounds_pure(
                                matched_can_chars,
                                |src_idx| {
                                    if src_idx < extracted_data.char_bounds.len() {
                                        extracted_data.char_bounds[src_idx]
                                    } else {
                                        None
                                    }
                                },
                                page_width,
                                page_height,
                            );

                        let mut item_warnings = Vec::new();
                        if total_segment_count > 64 {
                            item_warnings.push(ScanWarning {
                                code: "SENSITIVE_SCAN_SEGMENT_LIMIT_EXCEEDED".into(),
                                message: "Match segment sayısı sınırı (64) aşıldı.".into(),
                            });
                            report.truncated = true;
                            if !has_segment_limit_warning {
                                has_segment_limit_warning = true;
                                push_report_warning(
                                    &mut report.warnings,
                                    "SENSITIVE_SCAN_SEGMENT_LIMIT_EXCEEDED",
                                    "Bir veya daha fazla eşleşmede segment sınırı aşıldı.",
                                );
                            }
                        }

                        let (final_status, item_geom, final_rects) =
                            resolve_item_geometry_and_rects(
                                calc_status,
                                page_geometry.clone(),
                                rects,
                                &mut item_warnings,
                            );

                        let text_preview = matched.as_str().chars().take(256).collect::<String>();
                        let push_res = push_sensitive_scan_item(
                            &mut report,
                            SensitiveScanItem {
                                page_index: page_index as u32,
                                pattern_index: pattern_index as u32,
                                text_preview,
                                geometry_status: final_status,
                                page_geometry: item_geom,
                                source_rects_pdf_points: final_rects,
                                warnings: item_warnings,
                            },
                        );

                        if push_res == ScanItemPushResult::LimitExceeded {
                            break 'page_loop;
                        }
                    }
                }
            }

            if hit_text_limit {
                break 'page_loop;
            }
        }

        Ok(report)
    })
}

fn push_report_warning(warnings: &mut Vec<ScanWarning>, code: &str, message: &str) {
    if !warnings.iter().any(|w| w.code == code) {
        warnings.push(ScanWarning {
            code: code.to_string(),
            message: message.to_string(),
        });
    }
}

fn apply_redactions_unavailable(
) -> Result<SessionJobOutcome<()>, crate::document_session::DocumentSessionError> {
    Err(crate::document_session::DocumentSessionError::new(
        "FEATURE_UNAVAILABLE",
        "Güvenli redaksiyon bu sürümde devre dışıdır",
    ))
}

fn sanitize_pdf_document_unavailable(
) -> Result<SessionJobOutcome<()>, crate::document_session::DocumentSessionError> {
    Err(crate::document_session::DocumentSessionError::new(
        "FEATURE_UNAVAILABLE",
        "PDF sanitization bu sürümde devre dışıdır",
    ))
}

#[tauri::command]
pub async fn apply_redactions(
    _job_id: String,
    _session_id: String,
    _expected_content_generation: u64,
    _output_path: String,
    _redactions: Vec<RedactionArea>,
    _store: tauri::State<'_, crate::document_session::DocumentSessionStore>,
) -> Result<SessionJobOutcome<()>, crate::document_session::DocumentSessionError> {
    apply_redactions_unavailable()
}

#[tauri::command]
pub async fn sanitize_pdf_document(
    _job_id: String,
    _session_id: String,
    _expected_content_generation: u64,
    _output_path: String,
    _options: SanitizationOptions,
    _store: tauri::State<'_, crate::document_session::DocumentSessionStore>,
) -> Result<SessionJobOutcome<()>, crate::document_session::DocumentSessionError> {
    sanitize_pdf_document_unavailable()
}

#[tauri::command]
pub async fn scan_sensitive_data(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    patterns: Vec<String>,
    store: tauri::State<'_, crate::document_session::DocumentSessionStore>,
) -> Result<SessionJobOutcome<SensitiveScanReport>, crate::document_session::DocumentSessionError> {
    let _compiled = validate_patterns(&patterns).map_err(|err| match err {
        SessionJobOperationError::Failed(f) => {
            crate::document_session::DocumentSessionError::new(f.code, f.message)
        }
        SessionJobOperationError::Cancelled(c) => {
            crate::document_session::DocumentSessionError::new(
                "SESSION_JOB_CANCELLED",
                format!("{c:?}"),
            )
        }
    })?;

    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| match crate::job_worker::run_job(
                &context,
                crate::job_worker::JobCommand::ScanSensitiveData { patterns },
                std::time::Duration::from_secs(180),
            )? {
                crate::job_worker::JobResult::SensitiveReport(report) => Ok(report),
                _ => Err(SessionJobOperationError::failed_with_code(
                    "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                    "Sensitive-data worker beklenmeyen sonuç döndürdü",
                )),
            },
        )
        .await
}

#[tauri::command]
pub async fn plan_secure_redaction(
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    policy: crate::secure_redaction::SecureRedactionPolicy,
    acknowledgements: crate::secure_redaction::RedactionAcknowledgements,
    regions: Vec<crate::secure_redaction::RedactionRegionInput>,
    store: tauri::State<'_, crate::document_session::DocumentSessionStore>,
) -> Result<
    SessionJobOutcome<crate::secure_redaction::SecureRedactionPlan>,
    crate::document_session::DocumentSessionError,
> {
    let map_op_err = |err: SessionJobOperationError| match err {
        SessionJobOperationError::Failed(f) => {
            crate::document_session::DocumentSessionError::new(f.code, f.message)
        }
        SessionJobOperationError::Cancelled(c) => {
            crate::document_session::DocumentSessionError::new(
                "SESSION_JOB_CANCELLED",
                format!("{c:?}"),
            )
        }
    };

    crate::secure_redaction::validate_secure_redaction_policy(&policy).map_err(map_op_err)?;
    crate::secure_redaction::validate_redaction_acknowledgements(&acknowledgements)
        .map_err(map_op_err)?;

    crate::secure_redaction::validate_redaction_region_inputs(&regions).map_err(map_op_err)?;

    store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                let content_generation = context.captured_content_generation;
                match crate::job_worker::run_job(
                    &context,
                    crate::job_worker::JobCommand::PlanSecureRedaction {
                        content_generation,
                        policy,
                        acknowledgements,
                        regions,
                    },
                    std::time::Duration::from_secs(180),
                )? {
                    crate::job_worker::JobResult::SecureRedactionPlan(plan) => Ok(plan),
                    _ => Err(SessionJobOperationError::failed_with_code(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "Job worker beklenmeyen sonuç döndürdü",
                    )),
                }
            },
        )
        .await
}

#[derive(Clone, Debug)]
struct SecureRedactionPublicationTarget {
    canonical_target: PathBuf,
    expectation: crate::document_io::SecurePublicationTargetExpectation,
}

/// Deterministic acceptance boundaries for the secure-redaction orchestrator.
///
/// This support surface is never selected by the Tauri command or the normal
/// core entry point. It exists so integration tests can stop the real worker
/// chain at an exact boundary without environment variables or timing guesses.
#[cfg(feature = "secure-redaction-test-support")]
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecureRedactionExecutionTestPhase {
    AfterSnapshotCreated,
    BeforeTransform,
    TransformWorkerStarted,
    AfterTransformValidated,
    BeforeVerifier,
    VerifierWorkerStarted,
    AfterVerifierValidated,
    BeforePublication,
    PublicationStarted,
    AfterPublication,
}

#[cfg(not(feature = "secure-redaction-test-support"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SecureRedactionExecutionTestPhase {
    AfterSnapshotCreated,
    BeforeTransform,
    TransformWorkerStarted,
    AfterTransformValidated,
    BeforeVerifier,
    VerifierWorkerStarted,
    AfterVerifierValidated,
    BeforePublication,
    PublicationStarted,
    AfterPublication,
}

#[cfg(feature = "secure-redaction-test-support")]
#[doc(hidden)]
#[derive(Clone, Debug, Default)]
pub enum SecureRedactionWorkerTestAction {
    #[default]
    Real,
    Fail {
        code: String,
        message: String,
    },
    Timeout,
    HangUntilCancelled,
}

#[cfg(feature = "secure-redaction-test-support")]
#[doc(hidden)]
pub type SecureRedactionPhaseTestHook =
    Arc<dyn Fn(SecureRedactionExecutionTestPhase, Option<&Path>) + Send + Sync + 'static>;

#[cfg(feature = "secure-redaction-test-support")]
#[doc(hidden)]
pub type SecureRedactionResultTestHook = Arc<
    dyn Fn(crate::job_worker::JobResult) -> crate::job_worker::JobResult + Send + Sync + 'static,
>;

#[cfg(feature = "secure-redaction-test-support")]
#[doc(hidden)]
#[derive(Clone, Default)]
pub struct SecureRedactionExecutionTestSupport {
    pub phase_hook: Option<SecureRedactionPhaseTestHook>,
    pub transform_action: SecureRedactionWorkerTestAction,
    pub verifier_action: SecureRedactionWorkerTestAction,
    pub transform_result_hook: Option<SecureRedactionResultTestHook>,
    pub verifier_result_hook: Option<SecureRedactionResultTestHook>,
    pub corrupt_staged_temp: bool,
}

#[cfg(not(feature = "secure-redaction-test-support"))]
#[derive(Clone, Default)]
struct SecureRedactionExecutionTestSupport;

#[cfg(feature = "secure-redaction-test-support")]
fn notify_secure_redaction_test_phase(
    support: Option<&SecureRedactionExecutionTestSupport>,
    phase: SecureRedactionExecutionTestPhase,
    artifact: Option<&Path>,
) {
    if let Some(hook) = support.and_then(|support| support.phase_hook.as_ref()) {
        hook(phase, artifact);
    }
}

#[cfg(not(feature = "secure-redaction-test-support"))]
fn notify_secure_redaction_test_phase(
    _support: Option<&SecureRedactionExecutionTestSupport>,
    _phase: SecureRedactionExecutionTestPhase,
    _artifact: Option<&Path>,
) {
}

#[cfg(feature = "secure-redaction-test-support")]
fn run_secure_redaction_worker_for_execution(
    context: &crate::document_session::SessionJobContext,
    command: crate::job_worker::JobCommand,
    timeout: Duration,
    action: &SecureRedactionWorkerTestAction,
    test_support: Option<&SecureRedactionExecutionTestSupport>,
    started_phase: SecureRedactionExecutionTestPhase,
    artifact: Option<&Path>,
) -> Result<crate::job_worker::JobResult, SessionJobOperationError> {
    match action {
        SecureRedactionWorkerTestAction::Real => {
            crate::job_worker::run_job(context, command, timeout)
        }
        SecureRedactionWorkerTestAction::Fail { code, message } => {
            let mut child = controlled_secure_redaction_test_child(false);
            let status = context.child_processes.run_with_timeout(
                &mut child,
                Duration::from_secs(5),
                &context.cancellation,
            )?;
            if status.success() {
                return Err(SessionJobOperationError::failed_with_code(
                    "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                    "Controlled failure child unexpectedly succeeded",
                ));
            }
            Err(SessionJobOperationError::failed_with_code(
                code.clone(),
                message.clone(),
            ))
        }
        SecureRedactionWorkerTestAction::Timeout => {
            let mut child = controlled_secure_redaction_test_child(true);
            match context.child_processes.run_with_timeout(
                &mut child,
                Duration::from_millis(1),
                &context.cancellation,
            ) {
                Err(SessionJobOperationError::Cancelled(reason)) => {
                    Err(SessionJobOperationError::Cancelled(reason))
                }
                Err(_) => Err(SessionJobOperationError::failed_with_code(
                    "SESSION_JOB_WORKER_TIMEOUT",
                    "Controlled secure-redaction worker deadline expired",
                )),
                Ok(_) => Err(SessionJobOperationError::failed_with_code(
                    "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                    "Controlled timeout child unexpectedly exited before its deadline",
                )),
            }
        }
        SecureRedactionWorkerTestAction::HangUntilCancelled => {
            let mut child = controlled_secure_redaction_test_child(true);
            context.child_processes.run_with_timeout_after_spawn(
                &mut child,
                timeout,
                &context.cancellation,
                || notify_secure_redaction_test_phase(test_support, started_phase, artifact),
            )?;
            Err(SessionJobOperationError::failed_with_code(
                "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                "Controlled cancellation child unexpectedly exited",
            ))
        }
    }
}

#[cfg(not(feature = "secure-redaction-test-support"))]
fn run_secure_redaction_worker_for_execution(
    context: &crate::document_session::SessionJobContext,
    command: crate::job_worker::JobCommand,
    timeout: Duration,
    _action: &(),
    _test_support: Option<&SecureRedactionExecutionTestSupport>,
    _started_phase: SecureRedactionExecutionTestPhase,
    _artifact: Option<&Path>,
) -> Result<crate::job_worker::JobResult, SessionJobOperationError> {
    crate::job_worker::run_job(context, command, timeout)
}

#[cfg(feature = "secure-redaction-test-support")]
#[cfg(unix)]
fn controlled_secure_redaction_test_child(hang: bool) -> std::process::Command {
    let mut command = std::process::Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(if hang { "exec sleep 60" } else { "exit 23" })
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
}

#[cfg(feature = "secure-redaction-test-support")]
#[cfg(windows)]
fn controlled_secure_redaction_test_child(hang: bool) -> std::process::Command {
    let mut command = std::process::Command::new("cmd");
    command
        .arg("/C")
        .arg(if hang {
            "ping -n 60 127.0.0.1 >NUL"
        } else {
            "exit /B 23"
        })
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
}

fn secure_redaction_target_preflight(
    target_path: &str,
    paths: &crate::document_session::SecureRedactionExportPaths,
) -> Result<SecureRedactionPublicationTarget, crate::document_session::DocumentSessionError> {
    if target_path.trim().is_empty() || target_path.len() > 4096 {
        return Err(crate::document_session::DocumentSessionError::new(
            "REDACTION_TARGET_INVALID",
            "Secure redaction publication target geçerli bir yol olmalıdır",
        ));
    }
    let (canonical_target, expectation) =
        crate::document_io::capture_secure_publication_target_expectation(Path::new(target_path))
            .map_err(|detail| {
            crate::document_session::DocumentSessionError::new(
                "REDACTION_TARGET_PREFLIGHT_FAILED",
                detail,
            )
        })?;

    for protected in [&paths.original_path, &paths.working_path] {
        let same = crate::document_io::same_persistence_target(&canonical_target, protected)
            .map_err(|detail| {
                crate::document_session::DocumentSessionError::new(
                    "REDACTION_TARGET_PREFLIGHT_FAILED",
                    format!("Protected session path identity doğrulanamadı: {detail}"),
                )
            })?;
        if same {
            return Err(crate::document_session::DocumentSessionError::new(
                "REDACTION_TARGET_PROTECTED_PATH",
                "Secure redaction export original veya working PDF üzerine yazılamaz",
            ));
        }
    }

    for private_root in [&paths.session_dir, &paths.jobs_root] {
        let canonical_root = private_root.canonicalize().map_err(|error| {
            crate::document_session::DocumentSessionError::new(
                "REDACTION_TARGET_PREFLIGHT_FAILED",
                format!("Private session root çözümlenemedi: {error}"),
            )
        })?;
        if canonical_target.starts_with(&canonical_root) {
            return Err(crate::document_session::DocumentSessionError::new(
                "REDACTION_TARGET_PRIVATE_PATH",
                "Secure redaction export private session veya job dizinine yazılamaz",
            ));
        }
    }

    let parent = canonical_target.parent().ok_or_else(|| {
        crate::document_session::DocumentSessionError::new(
            "REDACTION_TARGET_PREFLIGHT_FAILED",
            "Secure redaction target parent dizini belirlenemedi",
        )
    })?;
    let parent_metadata = fs::metadata(parent).map_err(|error| {
        crate::document_session::DocumentSessionError::new(
            "REDACTION_TARGET_PREFLIGHT_FAILED",
            format!("Secure redaction target parent metadata okunamadı: {error}"),
        )
    })?;
    if !parent_metadata.is_dir() || parent_metadata.permissions().readonly() {
        return Err(crate::document_session::DocumentSessionError::new(
            "REDACTION_TARGET_PREFLIGHT_FAILED",
            "Secure redaction target parent kullanılabilir bir yazma dizini değil",
        ));
    }

    Ok(SecureRedactionPublicationTarget {
        canonical_target,
        expectation,
    })
}

fn secure_redaction_operation_error(
    code: &str,
    message: impl Into<String>,
) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code(code, message)
}

/// Authoritative arguments for secure redaction execution.
#[derive(Clone)]
pub struct SecureRedactionExecutionArgs<'a> {
    pub job_id: String,
    pub session_id: String,
    pub expected_content_generation: u64,
    pub policy: crate::secure_redaction::SecureRedactionPolicy,
    pub acknowledgements: crate::secure_redaction::RedactionAcknowledgements,
    pub regions: Vec<crate::secure_redaction::RedactionRegionInput>,
    pub expected_plan_digest: String,
    pub target_path: String,
    pub store: &'a crate::document_session::DocumentSessionStore,
}

impl<'a> std::fmt::Debug for SecureRedactionExecutionArgs<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecureRedactionExecutionArgs")
            .field("job_id", &self.job_id)
            .field("session_id", &self.session_id)
            .field(
                "expected_content_generation",
                &self.expected_content_generation,
            )
            .field("policy", &self.policy)
            .field("acknowledgements", &self.acknowledgements)
            .field("regions", &self.regions)
            .field("expected_plan_digest", &self.expected_plan_digest)
            .field("target_path", &self.target_path)
            .finish()
    }
}

/// Executes the authoritative secure-redaction export chain. The selected
/// target is deliberately consumed only by the main-process filesystem layer;
/// neither worker protocol variant contains it. The Tauri command below
/// intentionally contains no orchestration semantics beyond state
/// resolution and delegation, so acceptance tests can exercise this exact
/// production chain without an IPC wrapper.
pub async fn execute_secure_redaction_core(
    args: SecureRedactionExecutionArgs<'_>,
) -> Result<
    SessionJobOutcome<crate::secure_redaction::SecureRedactionExecutionReport>,
    crate::document_session::DocumentSessionError,
> {
    execute_secure_redaction_core_internal(args, None).await
}

#[cfg(feature = "secure-redaction-test-support")]
#[doc(hidden)]
pub async fn execute_secure_redaction_core_with_test_support(
    args: SecureRedactionExecutionArgs<'_>,
    test_support: Option<Arc<SecureRedactionExecutionTestSupport>>,
) -> Result<
    SessionJobOutcome<crate::secure_redaction::SecureRedactionExecutionReport>,
    crate::document_session::DocumentSessionError,
> {
    execute_secure_redaction_core_internal(args, test_support).await
}

async fn execute_secure_redaction_core_internal(
    args: SecureRedactionExecutionArgs<'_>,
    test_support: Option<Arc<SecureRedactionExecutionTestSupport>>,
) -> Result<
    SessionJobOutcome<crate::secure_redaction::SecureRedactionExecutionReport>,
    crate::document_session::DocumentSessionError,
> {
    let SecureRedactionExecutionArgs {
        job_id,
        session_id,
        expected_content_generation,
        policy,
        acknowledgements,
        regions,
        expected_plan_digest,
        target_path,
        store,
    } = args;
    let map_op_err = |err: SessionJobOperationError| match err {
        SessionJobOperationError::Failed(f) => {
            crate::document_session::DocumentSessionError::new(f.code, f.message)
        }
        SessionJobOperationError::Cancelled(c) => {
            crate::document_session::DocumentSessionError::new(
                "SESSION_JOB_CANCELLED",
                format!("{c:?}"),
            )
        }
    };
    crate::secure_redaction::validate_secure_redaction_policy(&policy).map_err(map_op_err)?;
    crate::secure_redaction::validate_redaction_acknowledgements(&acknowledgements)
        .map_err(map_op_err)?;
    crate::secure_redaction::validate_redaction_region_inputs(&regions).map_err(map_op_err)?;
    crate::secure_redaction::validate_sha256_digest(&expected_plan_digest, "expected_plan_digest")
        .map_err(map_op_err)?;

    // This read lease is held through atomic publication. Save / Save As take
    // the paired write lease, so the protected session persistence identities
    // cannot change after this authoritative preflight.
    let export_guard = store
        .acquire_secure_redaction_export_guard(&session_id, expected_content_generation)
        .await?;
    let publication_target = secure_redaction_target_preflight(&target_path, &export_guard.paths)?;

    let outcome = store
        .run_snapshot_job(
            &job_id,
            &session_id,
            expected_content_generation,
            move |context| {
                notify_secure_redaction_test_phase(
                    test_support.as_deref(),
                    SecureRedactionExecutionTestPhase::AfterSnapshotCreated,
                    None,
                );
                context.cancellation.check_cancelled()?;
                let input_digest = crate::document_io::sha256_file(&context.snapshot_path).map_err(|error| {
                    secure_redaction_operation_error(
                        "REDACTION_SNAPSHOT_DIGEST_FAILED",
                        format!("Immutable snapshot SHA-256 okunamadı: {error}"),
                    )
                })?;
                context.cancellation.check_cancelled()?;

                notify_secure_redaction_test_phase(
                    test_support.as_deref(),
                    SecureRedactionExecutionTestPhase::BeforeTransform,
                    None,
                );
                context.cancellation.check_cancelled()?;

                #[cfg(feature = "secure-redaction-test-support")]
                let transform_action = test_support
                    .as_ref()
                    .map(|support| support.transform_action.clone())
                    .unwrap_or_default();
                #[cfg(not(feature = "secure-redaction-test-support"))]
                let transform_action = ();
                #[cfg_attr(
                    not(feature = "secure-redaction-test-support"),
                    allow(unused_mut)
                )]
                let mut transform_result = run_secure_redaction_worker_for_execution(
                    &context,
                    crate::job_worker::JobCommand::ApplySecureRedaction {
                        content_generation: context.captured_content_generation,
                        policy: policy.clone(),
                        acknowledgements: acknowledgements.clone(),
                        regions: regions.clone(),
                        expected_input_digest: input_digest.clone(),
                        expected_plan_digest: expected_plan_digest.clone(),
                        output_name: crate::job_worker::SECURE_REDACTION_TRANSFORM_OUTPUT_NAME
                            .to_string(),
                    },
                    Duration::from_secs(300),
                    &transform_action,
                    test_support.as_deref(),
                    SecureRedactionExecutionTestPhase::TransformWorkerStarted,
                    None,
                )?;
                #[cfg(feature = "secure-redaction-test-support")]
                if let Some(hook) = test_support
                    .as_ref()
                    .and_then(|support| support.transform_result_hook.as_ref())
                {
                    transform_result = hook(transform_result);
                }
                let transform = match transform_result {
                    crate::job_worker::JobResult::SecureRedactionTransform(report) => report,
                    _ => return Err(secure_redaction_operation_error(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "Secure redaction transform worker beklenmeyen sonuç döndürdü",
                    )),
                };
                crate::secure_redaction::validate_transform_report(&transform)?;
                if transform.job_id != context.job_id.to_string()
                    || transform.session_id != context.owner_session_id
                    || transform.input_digest != input_digest
                    || transform.plan_digest != expected_plan_digest
                {
                    return Err(secure_redaction_operation_error(
                        "REDACTION_TRANSFORM_REPORT_MISMATCH",
                        "Transform report authoritative session, snapshot veya plan bağını karşılamıyor",
                    ));
                }
                let artifact = crate::job_worker::validate_output_file(
                    &context,
                    &context.snapshot_path.parent().unwrap().join(&transform.output_relative_name),
                )?;
                let artifact_size = fs::metadata(&artifact).map_err(|error| {
                    secure_redaction_operation_error(
                        "REDACTION_TRANSFORM_ARTIFACT_INVALID",
                        format!("Transform artifact metadata okunamadı: {error}"),
                    )
                })?.len();
                if artifact_size != transform.output_size_bytes {
                    return Err(secure_redaction_operation_error(
                        "REDACTION_TRANSFORM_ARTIFACT_INVALID",
                        "Transform artifact boyutu typed transform report ile eşleşmiyor",
                    ));
                }
                let artifact_digest = crate::document_io::sha256_file(&artifact).map_err(|error| {
                    secure_redaction_operation_error(
                        "REDACTION_TRANSFORM_ARTIFACT_INVALID",
                        format!("Transform artifact SHA-256 okunamadı: {error}"),
                    )
                })?;
                if artifact_digest != transform.output_digest {
                    return Err(secure_redaction_operation_error(
                        "REDACTION_TRANSFORM_ARTIFACT_DIGEST_MISMATCH",
                        "Transform artifact digest'i typed transform report ile eşleşmiyor",
                    ));
                }
                notify_secure_redaction_test_phase(
                    test_support.as_deref(),
                    SecureRedactionExecutionTestPhase::AfterTransformValidated,
                    Some(&artifact),
                );
                context.cancellation.check_cancelled()?;

                notify_secure_redaction_test_phase(
                    test_support.as_deref(),
                    SecureRedactionExecutionTestPhase::BeforeVerifier,
                    Some(&artifact),
                );
                context.cancellation.check_cancelled()?;

                #[cfg(feature = "secure-redaction-test-support")]
                let verifier_action = test_support
                    .as_ref()
                    .map(|support| support.verifier_action.clone())
                    .unwrap_or_default();
                #[cfg(not(feature = "secure-redaction-test-support"))]
                let verifier_action = ();
                #[cfg_attr(
                    not(feature = "secure-redaction-test-support"),
                    allow(unused_mut)
                )]
                let mut verifier_result = run_secure_redaction_worker_for_execution(
                    &context,
                    crate::job_worker::JobCommand::VerifySecureRedaction {
                        content_generation: context.captured_content_generation,
                        policy: policy.clone(),
                        acknowledgements: acknowledgements.clone(),
                        regions: regions.clone(),
                        transform_report: transform.clone(),
                    },
                    Duration::from_secs(300),
                    &verifier_action,
                    test_support.as_deref(),
                    SecureRedactionExecutionTestPhase::VerifierWorkerStarted,
                    Some(&artifact),
                )?;
                #[cfg(feature = "secure-redaction-test-support")]
                if let Some(hook) = test_support
                    .as_ref()
                    .and_then(|support| support.verifier_result_hook.as_ref())
                {
                    verifier_result = hook(verifier_result);
                }
                let verification = match verifier_result {
                    crate::job_worker::JobResult::SecureRedactionVerification(report) => report,
                    _ => return Err(secure_redaction_operation_error(
                        "SESSION_JOB_WORKER_PROTOCOL_ERROR",
                        "Secure redaction verification worker beklenmeyen sonuç döndürdü",
                    )),
                };
                crate::secure_redaction::validate_verification_report(&verification)?;
                if verification.job_id != context.job_id.to_string()
                    || verification.session_id != context.owner_session_id
                    || verification.plan_digest != transform.plan_digest
                    || verification.transform_digest != transform.output_digest
                    || !verification.is_verified
                    || !verification.blockers.is_empty()
                    || verification.verification_checks.iter().any(|check| {
                        check.status != crate::secure_redaction::VerificationCheckStatus::Passed
                    })
                {
                    return Err(secure_redaction_operation_error(
                        "REDACTION_VERIFICATION_GATE_FAILED",
                        "Independent verification publication için zorunlu koşulları karşılamıyor",
                    ));
                }
                notify_secure_redaction_test_phase(
                    test_support.as_deref(),
                    SecureRedactionExecutionTestPhase::AfterVerifierValidated,
                    Some(&artifact),
                );
                context.cancellation.check_cancelled()?;

                notify_secure_redaction_test_phase(
                    test_support.as_deref(),
                    SecureRedactionExecutionTestPhase::BeforePublication,
                    Some(&artifact),
                );
                context.cancellation.check_cancelled()?;

                // Cancellation is intentionally checked before entering the
                // atomic critical section. Once document_io starts replacement,
                // we observe its disk-truth result instead of reporting a false
                // cancellation.
                notify_secure_redaction_test_phase(
                    test_support.as_deref(),
                    SecureRedactionExecutionTestPhase::PublicationStarted,
                    Some(&artifact),
                );
                #[cfg(feature = "secure-redaction-test-support")]
                let commit_result = if let Some(support) = test_support.as_ref() {
                    crate::document_io::atomic_publish_prevalidated_pdf_for_acceptance(
                        "secure_redaction_publish",
                        &context.snapshot_path,
                        &publication_target.canonical_target,
                        &artifact,
                        &verification.transform_digest,
                        publication_target.expectation.clone(),
                        support.corrupt_staged_temp,
                    )
                } else {
                    crate::document_io::atomic_publish_prevalidated_pdf(
                        "secure_redaction_publish",
                        &context.snapshot_path,
                        &publication_target.canonical_target,
                        &artifact,
                        &verification.transform_digest,
                        publication_target.expectation.clone(),
                    )
                };
                #[cfg(not(feature = "secure-redaction-test-support"))]
                let commit_result = crate::document_io::atomic_publish_prevalidated_pdf(
                    "secure_redaction_publish",
                    &context.snapshot_path,
                    &publication_target.canonical_target,
                    &artifact,
                    &verification.transform_digest,
                    publication_target.expectation.clone(),
                );
                let commit = commit_result.map_err(|error| {
                    let code = if error.detail.contains("REDACTION_TARGET_CONFLICT")
                        || error.detail.contains("PRECONDITION_TARGET_CHANGED") {
                        "REDACTION_TARGET_CONFLICT"
                    } else if error.detail.contains("REDACTION_VERIFIED_ARTIFACT_DIGEST_MISMATCH") {
                        "REDACTION_VERIFIED_ARTIFACT_DIGEST_MISMATCH"
                    } else {
                        "REDACTION_PUBLICATION_FAILED"
                    };
                    SessionJobOperationError::Failed(
                        crate::document_session::SessionJobFailure::publication_failure(code, error),
                    )
                })?;
                notify_secure_redaction_test_phase(
                    test_support.as_deref(),
                    SecureRedactionExecutionTestPhase::AfterPublication,
                    Some(&artifact),
                );
                if commit.committed_digest != verification.transform_digest {
                    return Err(secure_redaction_operation_error(
                        "REDACTION_PUBLICATION_DIGEST_MISMATCH",
                        "Atomic publication committed digest verifier digest'iyle eşleşmiyor",
                    ));
                }
                let report = crate::secure_redaction::SecureRedactionExecutionReport {
                    schema_version: crate::secure_redaction::REDACTION_SCHEMA_VERSION.to_string(),
                    job_id: context.job_id.to_string(),
                    session_id: context.owner_session_id.clone(),
                    transform_report: transform,
                    verification_report: verification,
                    published: true,
                    publication_report: Some(crate::secure_redaction::SecureRedactionPublicationReport {
                        schema_version: crate::secure_redaction::REDACTION_SCHEMA_VERSION.to_string(),
                        publication_id: format!("secure-redaction-{}", context.job_id),
                        published_path: publication_target.canonical_target.to_string_lossy().into_owned(),
                        published_digest: commit.committed_digest,
                        commit_state: crate::secure_redaction::SecureRedactionCommitState::from_document_io(commit.state),
                    }),
                };
                crate::secure_redaction::validate_execution_report(&report)?;
                Ok(report)
            },
        )
        .await;
    drop(export_guard);
    outcome
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecuteSecureRedactionPayload {
    job_id: String,
    session_id: String,
    expected_content_generation: u64,
    policy: crate::secure_redaction::SecureRedactionPolicy,
    acknowledgements: crate::secure_redaction::RedactionAcknowledgements,
    regions: Vec<crate::secure_redaction::RedactionRegionInput>,
    expected_plan_digest: String,
    target_path: String,
}

#[tauri::command]
pub async fn execute_secure_redaction(
    request: tauri::ipc::Request<'_>,
    store: tauri::State<'_, crate::document_session::DocumentSessionStore>,
) -> Result<
    SessionJobOutcome<crate::secure_redaction::SecureRedactionExecutionReport>,
    crate::document_session::DocumentSessionError,
> {
    let payload: ExecuteSecureRedactionPayload = match request.body() {
        tauri::ipc::InvokeBody::Json(value) => {
            serde_json::from_value(value.clone()).map_err(|err| {
                crate::document_session::DocumentSessionError::new(
                    "INVALID_COMMAND_PAYLOAD",
                    format!("Failed to parse payload: {err}"),
                )
            })?
        }
        tauri::ipc::InvokeBody::Raw(bytes) => serde_json::from_slice(bytes).map_err(|err| {
            crate::document_session::DocumentSessionError::new(
                "INVALID_COMMAND_PAYLOAD",
                format!("Failed to parse payload: {err}"),
            )
        })?,
    };

    execute_secure_redaction_core(SecureRedactionExecutionArgs {
        job_id: payload.job_id,
        session_id: payload.session_id,
        expected_content_generation: payload.expected_content_generation,
        policy: payload.policy,
        acknowledgements: payload.acknowledgements,
        regions: payload.regions,
        expected_plan_digest: payload.expected_plan_digest,
        target_path: payload.target_path,
        store: &store,
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("pdf")
            .join(name)
    }

    #[test]
    fn test_apply_redactions_unavailable() {
        let res = apply_redactions_unavailable();
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.code, "FEATURE_UNAVAILABLE");
    }

    #[test]
    fn test_sanitize_pdf_document_unavailable() {
        let res = sanitize_pdf_document_unavailable();
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert_eq!(err.code, "FEATURE_UNAVAILABLE");
    }

    #[test]
    fn test_validate_patterns_limits_and_syntax() {
        let valid = vec![
            "\\b[0-9]{11}\\b".to_string(),
            "test@domain\\.com".to_string(),
        ];
        assert!(validate_patterns(&valid).is_ok());

        let invalid = vec!["valid".to_string(), "[unclosed".to_string()];
        let err = validate_patterns(&invalid).unwrap_err();
        if let SessionJobOperationError::Failed(f) = err {
            assert_eq!(f.code, "SENSITIVE_SCAN_INVALID_PATTERN");
            assert_eq!(
                f.message,
                "Pattern index 1 geçersiz regex sözdizimi içeriyor."
            );
            assert!(!f.message.contains("[unclosed"));
        } else {
            panic!("Expected SessionJobOperationError::Failed");
        }

        let too_many = vec!["pattern".to_string(); 65];
        let err = validate_patterns(&too_many).unwrap_err();
        if let SessionJobOperationError::Failed(f) = err {
            assert_eq!(f.code, "SENSITIVE_SCAN_PATTERN_LIMIT_EXCEEDED");
        } else {
            panic!("Expected SessionJobOperationError::Failed");
        }

        let huge_pattern = "a".repeat(1025);
        let err = validate_patterns(&[huge_pattern]).unwrap_err();
        if let SessionJobOperationError::Failed(f) = err {
            assert_eq!(f.code, "SENSITIVE_SCAN_PATTERN_LIMIT_EXCEEDED");
        } else {
            panic!("Expected SessionJobOperationError::Failed");
        }

        // 64 patterns * 512 bytes = 32 KiB total -> OK
        let patterns_32k = vec!["a".repeat(512); 64];
        assert!(validate_patterns(&patterns_32k).is_ok());

        // 64 patterns totalling 32 KiB + 1 byte -> EXCEEDED
        let mut patterns_over_32k = vec!["a".repeat(512); 63];
        patterns_over_32k.push("a".repeat(513)); // 513 <= 1024 individually, total = 32769 > 32768
        let err = validate_patterns(&patterns_over_32k).unwrap_err();
        if let SessionJobOperationError::Failed(f) = err {
            assert_eq!(f.code, "SENSITIVE_SCAN_PATTERN_LIMIT_EXCEEDED");
            assert_eq!(f.message, "Toplam pattern boyutu 32 KiB sınırını aşamaz");
        } else {
            panic!("Expected SessionJobOperationError::Failed");
        }
    }

    #[test]
    fn test_text_limit_exact_and_overflow_semantics() {
        let make_chars = |n: usize| {
            (0..n).map(|i| PureSourceChar {
                source_index: i,
                unicode: Some('A'),
                bounds: Some((i as f32, 0.0, (i + 1) as f32, 10.0)),
            })
        };

        // Exactly limit chars (input = 5, max = 5) -> hit_text_limit = false
        let mut count = 0;
        let (data_exact, hit_exact) = build_canonical_runs_pure(make_chars(5), &mut count, 5);
        assert!(!hit_exact);
        assert_eq!(count, 5);
        assert_eq!(data_exact.runs[0].text, "AAAAA");
        assert_eq!(data_exact.char_bounds.len(), 5);

        // Limit chars + 1 extra (input = 6, max = 5) -> hit_text_limit = true
        let mut count2 = 0;
        let (data_overflow, hit_overflow) =
            build_canonical_runs_pure(make_chars(6), &mut count2, 5);
        assert!(hit_overflow);
        assert_eq!(count2, 5);
        assert_eq!(data_overflow.runs[0].text, "AAAAA");
        assert_eq!(data_overflow.char_bounds.len(), 5); // Only 5 bounds contained
    }

    #[test]
    fn test_calculate_scan_page_bounds() {
        assert_eq!(calculate_scan_page_bounds(10_000, 10_000), (10_000, false));
        assert_eq!(calculate_scan_page_bounds(10_001, 10_000), (10_000, true));
        assert_eq!(calculate_scan_page_bounds(5, 10_000), (5, false));
    }

    #[test]
    fn test_canonical_run_unicode_error_splits_runs() {
        let input_chars = vec![
            PureSourceChar {
                source_index: 0,
                unicode: Some('A'),
                bounds: None,
            },
            PureSourceChar {
                source_index: 1,
                unicode: Some('B'),
                bounds: None,
            },
            PureSourceChar {
                source_index: 2,
                unicode: None,
                bounds: None,
            }, // error
            PureSourceChar {
                source_index: 3,
                unicode: Some('C'),
                bounds: None,
            },
            PureSourceChar {
                source_index: 4,
                unicode: Some('D'),
                bounds: None,
            },
        ];

        let mut count = 0;
        let (data, hit_limit) = build_canonical_runs_pure(input_chars.into_iter(), &mut count, 100);
        assert!(!hit_limit);
        assert_eq!(count, 5); // unicode_char == None also counted towards processed count!
        assert_eq!(data.runs.len(), 2);
        assert_eq!(data.runs[0].text, "AB");
        assert_eq!(data.runs[1].text, "CD");

        let re = Regex::new("BC").unwrap();
        let mut matches = 0;
        for run in &data.runs {
            matches += re.find_iter(&run.text).count();
        }
        assert_eq!(matches, 0); // match cannot form across runs
    }

    #[test]
    fn test_unresolvable_unicode_increments_processed_count() {
        let input_chars = vec![
            PureSourceChar {
                source_index: 0,
                unicode: None,
                bounds: None,
            },
            PureSourceChar {
                source_index: 1,
                unicode: None,
                bounds: None,
            },
            PureSourceChar {
                source_index: 2,
                unicode: Some('X'),
                bounds: None,
            },
        ];

        let mut count = 0;
        let (data, hit_limit) = build_canonical_runs_pure(input_chars.into_iter(), &mut count, 2);
        assert!(hit_limit);
        assert_eq!(count, 2); // stopped after 2 source chars even though they were None!
        assert!(data.runs.is_empty());
    }

    #[test]
    fn test_canonical_run_turkish_and_emoji_binary_search_mapping() {
        let text_input = "Şebnem 🔒 12345678901 🎉";
        let input_chars: Vec<PureSourceChar> = text_input
            .chars()
            .enumerate()
            .map(|(i, c)| PureSourceChar {
                source_index: i,
                unicode: Some(c),
                bounds: Some((i as f32, 0.0, (i + 1) as f32, 10.0)),
            })
            .collect();

        let mut count = 0;
        let (data, _) = build_canonical_runs_pure(input_chars.into_iter(), &mut count, 1000);
        let run = &data.runs[0];

        // Invariant check: byte_boundaries.len() == characters.len() + 1, first element 0
        assert_eq!(run.byte_boundaries.len(), run.characters.len() + 1);
        assert_eq!(run.byte_boundaries[0], 0);

        let re = Regex::new(r"\d{11}").unwrap();
        let matched = re.find(&run.text).unwrap();

        let start_res = run.byte_boundaries.binary_search(&matched.start());
        let end_res = run.byte_boundaries.binary_search(&matched.end());

        assert!(start_res.is_ok());
        assert!(end_res.is_ok());

        let s = start_res.unwrap();
        let e = end_res.unwrap();

        let matched_chars: String = run.characters[s..e].iter().map(|c| c.value).collect();
        assert_eq!(matched_chars, "12345678901");
    }

    #[test]
    fn test_vertical_overlap_50_percent_threshold_and_staircase() {
        let chars = vec![
            CanonicalCharacter {
                value: 'A',
                source_character_index: 0,
            },
            CanonicalCharacter {
                value: 'B',
                source_character_index: 1,
            },
            CanonicalCharacter {
                value: 'C',
                source_character_index: 2,
            },
        ];

        // 1. Overlap >= 50% between adjacent chars
        let get_bounds_70 = |idx| match idx {
            0 => Some((0.0, 0.0, 10.0, 10.0)),
            1 => Some((10.0, 3.0, 20.0, 13.0)),
            2 => Some((20.0, 6.0, 30.0, 16.0)),
            _ => None,
        };

        let (status, rects, seg_count) =
            compute_segments_from_bounds_pure(&chars, get_bounds_70, 100.0, 100.0);
        assert_eq!(status, GeometryStatus::Resolved);
        assert_eq!(seg_count, 1);
        assert_eq!(rects.len(), 1);
        assert_eq!(
            rects[0],
            PdfRect {
                x: 0.0,
                y: 0.0,
                width: 30.0,
                height: 16.0
            }
        );

        // 2. Vertical Staircase Test
        let get_bounds_staircase = |idx| match idx {
            0 => Some((0.0, 0.0, 10.0, 10.0)),
            1 => Some((10.0, 4.0, 20.0, 14.0)),
            2 => Some((0.0, 12.0, 10.0, 22.0)),
            _ => None,
        };

        let (status, rects, seg_count) =
            compute_segments_from_bounds_pure(&chars, get_bounds_staircase, 100.0, 100.0);
        assert_eq!(status, GeometryStatus::Resolved);
        assert_eq!(seg_count, 2);
        assert_eq!(rects.len(), 2);
        assert_eq!(
            rects[0],
            PdfRect {
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 14.0
            }
        );
        assert_eq!(
            rects[1],
            PdfRect {
                x: 0.0,
                y: 12.0,
                width: 10.0,
                height: 10.0
            }
        );
    }

    #[test]
    fn test_segment_limit_64_resolved_65_unresolved() {
        let chars_64: Vec<CanonicalCharacter> = (0..64)
            .map(|i| CanonicalCharacter {
                value: 'x',
                source_character_index: i,
            })
            .collect();

        let get_bounds_64 = |idx| Some((0.0, (idx * 20) as f32, 10.0, ((idx * 20) + 10) as f32));
        let (status64, rects64, count64) =
            compute_segments_from_bounds_pure(&chars_64, get_bounds_64, 1000.0, 2000.0);
        assert_eq!(status64, GeometryStatus::Resolved);
        assert_eq!(count64, 64);
        assert_eq!(rects64.len(), 64);

        let chars_65: Vec<CanonicalCharacter> = (0..65)
            .map(|i| CanonicalCharacter {
                value: 'x',
                source_character_index: i,
            })
            .collect();

        let (status65, rects65, count65) =
            compute_segments_from_bounds_pure(&chars_65, get_bounds_64, 1000.0, 2000.0);
        assert_eq!(status65, GeometryStatus::Unresolved);
        assert_eq!(count65, 65);
        assert!(rects65.is_empty());
    }

    #[test]
    fn test_exact_10k_matches_and_10001st_match_truncation() {
        let mut report = SensitiveScanReport {
            schema_version: 2,
            items: Vec::new(),
            warnings: Vec::new(),
            truncated: false,
        };

        let sample_item = SensitiveScanItem {
            page_index: 0,
            pattern_index: 0,
            text_preview: "X".into(),
            geometry_status: GeometryStatus::Unresolved,
            page_geometry: None,
            source_rects_pdf_points: vec![],
            warnings: vec![],
        };

        for _ in 0..10_000 {
            let res = push_sensitive_scan_item(&mut report, sample_item.clone());
            assert_eq!(res, ScanItemPushResult::Added);
        }

        assert_eq!(report.items.len(), 10_000);
        assert!(!report.truncated);
        assert!(report.warnings.is_empty());

        let extra_res = push_sensitive_scan_item(&mut report, sample_item.clone());
        assert_eq!(extra_res, ScanItemPushResult::LimitExceeded);
        assert_eq!(report.items.len(), 10_000);
        assert!(report.truncated);
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(
            report.warnings[0].code,
            "SENSITIVE_SCAN_ITEM_LIMIT_EXCEEDED"
        );
    }

    #[test]
    fn test_fake_rect_is_never_produced() {
        let chars = vec![CanonicalCharacter {
            value: 'X',
            source_character_index: 0,
        }];

        let get_bounds_none = |_| None;
        let (status_none, rects_none, _) =
            compute_segments_from_bounds_pure(&chars, get_bounds_none, 500.0, 500.0);
        assert_eq!(status_none, GeometryStatus::Unresolved);
        assert!(rects_none.is_empty());

        let get_bounds_zero_width = |_| Some((0.0, 0.0, 0.0, 10.0));
        let (status_zero, rects_zero, _) =
            compute_segments_from_bounds_pure(&chars, get_bounds_zero_width, 500.0, 500.0);
        assert_eq!(status_zero, GeometryStatus::Unresolved);
        assert!(rects_zero.is_empty());
    }

    #[test]
    fn test_intrinsic_rotation_numeric_serde() {
        let rot = IntrinsicRotation::Deg90;
        let json = serde_json::to_string(&rot).unwrap();
        assert_eq!(json, "90");

        let de: IntrinsicRotation = serde_json::from_str("180").unwrap();
        assert_eq!(de, IntrinsicRotation::Deg180);

        assert!(serde_json::from_str::<IntrinsicRotation>("45").is_err());
        assert!(serde_json::from_str::<IntrinsicRotation>("\"90\"").is_err());
        assert!(serde_json::from_str::<IntrinsicRotation>("\"Rotate90\"").is_err());
    }

    #[test]
    fn test_extract_page_geometry_and_schema_v2() {
        crate::pdf_engine::with_pdfium(|pdfium| {
            let doc = pdfium
                .load_pdf_from_file(&fixture_path("reorder-rich.pdf"), None)
                .unwrap();

            let page0 = doc.pages().get(0).unwrap();
            let geom0 = extract_page_geometry(pdfium, &page0).unwrap();
            assert_eq!(geom0.coordinate_space, CoordinateSpace::PdfUserSpace);
            assert_eq!(geom0.unit, GeometryUnit::PdfPoint);
            assert_eq!(geom0.origin, BoxOrigin::BottomLeft);
            assert_eq!(geom0.effective_box.left, 0.0);
            assert_eq!(geom0.effective_box.bottom, 0.0);
            assert_eq!(geom0.effective_box.width, 300.0);
            assert_eq!(geom0.effective_box.height, 400.0);
            assert_eq!(geom0.intrinsic_rotation, IntrinsicRotation::Deg0);

            let page1 = doc.pages().get(1).unwrap();
            let geom1 = extract_page_geometry(pdfium, &page1).unwrap();
            assert_eq!(geom1.effective_box.left, 0.0);
            assert_eq!(geom1.effective_box.bottom, 0.0);
            assert_eq!(geom1.effective_box.width, 400.0);
            assert_eq!(geom1.effective_box.height, 500.0);
            assert_eq!(geom1.intrinsic_rotation, IntrinsicRotation::Deg90);
            Ok::<(), String>(())
        })
        .unwrap();
    }

    fn build_geometry_fixture_pdf(
        media_box: (f32, f32, f32, f32),
        crop_box: Option<(f32, f32, f32, f32)>,
        rotation: i32,
        text: &str,
    ) -> tempfile::NamedTempFile {
        use lopdf::content::{Content, Operation};
        use lopdf::{dictionary, Document, Object, Stream, StringFormat};

        let mut doc = Document::with_version("1.7");
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });

        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec![Object::Name(b"F1".to_vec()), 12.into()]),
                Operation::new("Td", vec![50.into(), 100.into()]),
                Operation::new(
                    "Tj",
                    vec![Object::String(
                        text.as_bytes().to_vec(),
                        StringFormat::Literal,
                    )],
                ),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));

        let mut page_dict = dictionary! {
            "Type" => "Page",
            "MediaBox" => vec![
                media_box.0.into(),
                media_box.1.into(),
                media_box.2.into(),
                media_box.3.into(),
            ],
            "Rotate" => rotation,
            "Contents" => content_id,
            "Resources" => dictionary! {
                "Font" => dictionary! { "F1" => font_id },
            },
        };

        if let Some(crop) = crop_box {
            page_dict.set(
                "CropBox",
                vec![crop.0.into(), crop.1.into(), crop.2.into(), crop.3.into()],
            );
        }

        let page_id = doc.add_object(page_dict);
        let pages_id = doc.add_object(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
        });

        if let Ok(Object::Dictionary(ref mut page_obj)) = doc.get_object_mut(page_id) {
            page_obj.set("Parent", pages_id);
        }

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });

        doc.trailer.set("Root", catalog_id);

        let temp_file = tempfile::NamedTempFile::new().unwrap();
        doc.save(temp_file.path()).unwrap();
        temp_file
    }

    #[test]
    fn test_text_preview_256_character_limit_and_multibyte_safety() {
        let exact_256 = "A".repeat(256);
        let over_256 = "B".repeat(300);

        let file256 = build_geometry_fixture_pdf((0.0, 0.0, 400.0, 600.0), None, 0, &exact_256);
        let file_over = build_geometry_fixture_pdf((0.0, 0.0, 400.0, 600.0), None, 0, &over_256);

        let report256 = scan_sensitive_file(file256.path(), None, &["A+".to_string()]).unwrap();
        assert_eq!(report256.items.len(), 1);
        assert_eq!(report256.items[0].text_preview.chars().count(), 256);
        assert_eq!(report256.items[0].text_preview, exact_256);

        let report_over = scan_sensitive_file(file_over.path(), None, &["B+".to_string()]).unwrap();
        assert_eq!(report_over.items.len(), 1);
        assert_eq!(report_over.items[0].text_preview.chars().count(), 256);
        assert_eq!(report_over.items[0].text_preview, "B".repeat(256));

        // Verify multi-byte UTF-8 character boundary safety without mid-character splitting
        let turkish_multibyte = "ĞŞİçöü🔥".repeat(40);
        let sliced_mb = turkish_multibyte.chars().take(256).collect::<String>();
        assert_eq!(sliced_mb.chars().count(), 256);
        assert!(sliced_mb.ends_with('ç'));
    }

    #[test]
    fn test_geometry_fixture_mediabox_cropbox_matrix() {
        crate::pdf_engine::with_pdfium(|pdfium| {
            // 1. MediaBox = CropBox, origin 0
            let f1 = build_geometry_fixture_pdf((0.0, 0.0, 500.0, 700.0), None, 0, "MATCH123");
            let doc1 = pdfium.load_pdf_from_file(f1.path(), None).unwrap();
            let geom1 = extract_page_geometry(pdfium, &doc1.pages().get(0).unwrap()).unwrap();
            assert_eq!(geom1.media_box.left, 0.0);
            assert_eq!(geom1.media_box.width, 500.0);
            assert_eq!(geom1.crop_box, None);
            assert_eq!(geom1.effective_box.left, 0.0);
            assert_eq!(geom1.effective_box.width, 500.0);

            // 2. CropBox different
            let f2 = build_geometry_fixture_pdf(
                (0.0, 0.0, 600.0, 800.0),
                Some((50.0, 100.0, 450.0, 650.0)),
                0,
                "MATCH123",
            );
            let doc2 = pdfium.load_pdf_from_file(f2.path(), None).unwrap();
            let geom2 = extract_page_geometry(pdfium, &doc2.pages().get(0).unwrap()).unwrap();
            assert_eq!(geom2.media_box.width, 600.0);
            let crop2 = geom2.crop_box.unwrap();
            assert_eq!(crop2.left, 50.0);
            assert_eq!(crop2.bottom, 100.0);
            assert_eq!(crop2.width, 400.0);
            assert_eq!(crop2.height, 550.0);
            assert_eq!(geom2.effective_box.left, 50.0);
            assert_eq!(geom2.effective_box.width, 400.0);

            // 3. Non-zero effective origin
            let f3 = build_geometry_fixture_pdf((100.0, 150.0, 600.0, 850.0), None, 0, "MATCH123");
            let doc3 = pdfium.load_pdf_from_file(f3.path(), None).unwrap();
            let geom3 = extract_page_geometry(pdfium, &doc3.pages().get(0).unwrap()).unwrap();
            assert_eq!(geom3.effective_box.left, 100.0);
            assert_eq!(geom3.effective_box.bottom, 150.0);

            // 4. Negative origin
            let f4 = build_geometry_fixture_pdf((-50.0, -40.0, 550.0, 760.0), None, 0, "MATCH123");
            let doc4 = pdfium.load_pdf_from_file(f4.path(), None).unwrap();
            let geom4 = extract_page_geometry(pdfium, &doc4.pages().get(0).unwrap()).unwrap();
            assert_eq!(geom4.effective_box.left, -50.0);
            assert_eq!(geom4.effective_box.bottom, -40.0);

            // 5. CropBox MediaBox overflow
            let f5 = build_geometry_fixture_pdf(
                (0.0, 0.0, 400.0, 600.0),
                Some((-20.0, -20.0, 500.0, 700.0)),
                0,
                "MATCH123",
            );
            let doc5 = pdfium.load_pdf_from_file(f5.path(), None).unwrap();
            let geom5 = extract_page_geometry(pdfium, &doc5.pages().get(0).unwrap()).unwrap();
            assert_eq!(geom5.effective_box.left, 0.0);
            assert_eq!(geom5.effective_box.bottom, 0.0);
            assert_eq!(geom5.effective_box.width, 400.0);
            assert_eq!(geom5.effective_box.height, 600.0);

            // 6. Intrinsic rotations 0, 90, 180, 270
            for (rot_deg, expected_variant) in [
                (0, IntrinsicRotation::Deg0),
                (90, IntrinsicRotation::Deg90),
                (180, IntrinsicRotation::Deg180),
                (270, IntrinsicRotation::Deg270),
            ] {
                let f =
                    build_geometry_fixture_pdf((0.0, 0.0, 400.0, 600.0), None, rot_deg, "MATCH123");
                let doc = pdfium.load_pdf_from_file(f.path(), None).unwrap();
                let geom = extract_page_geometry(pdfium, &doc.pages().get(0).unwrap()).unwrap();
                assert_eq!(geom.intrinsic_rotation, expected_variant);
            }

            // 7. Zero-width CropBox fail-closed returning None
            let f_zero_crop = build_geometry_fixture_pdf(
                (0.0, 0.0, 400.0, 600.0),
                Some((100.0, 0.0, 100.0, 600.0)),
                0,
                "MATCH123",
            );
            let doc_zero_crop = pdfium.load_pdf_from_file(f_zero_crop.path(), None).unwrap();
            assert!(
                extract_page_geometry(pdfium, &doc_zero_crop.pages().get(0).unwrap()).is_none()
            );

            Ok::<(), String>(())
        })
        .unwrap();
    }

    #[test]
    fn test_scan_sensitive_file_full_schema_v2_geometry_matrix() {
        let f = build_geometry_fixture_pdf((0.0, 0.0, 400.0, 600.0), None, 0, "MATCH123");
        let report = scan_sensitive_file(f.path(), None, &["MATCH\\d+".to_string()]).unwrap();

        assert_eq!(report.schema_version, 2);
        assert_eq!(report.items.len(), 1);
        let item = &report.items[0];
        assert_eq!(item.geometry_status, GeometryStatus::Resolved);
        assert!(item.page_geometry.is_some());
        assert!(!item.source_rects_pdf_points.is_empty());
        let geom = item.page_geometry.as_ref().unwrap();
        assert_eq!(geom.coordinate_space, CoordinateSpace::PdfUserSpace);
        assert_eq!(geom.unit, GeometryUnit::PdfPoint);
        assert_eq!(geom.origin, BoxOrigin::BottomLeft);
    }

    #[test]
    fn test_scan_sensitive_file_minimal_pdf_no_match() {
        let report = scan_sensitive_file(
            &fixture_path("minimal.pdf"),
            None,
            &["\\bNONMATCHINGPATTERN12345\\b".to_string()],
        )
        .expect("Minimal PDF scan should succeed");

        assert_eq!(report.schema_version, 2);
        assert!(report.items.is_empty());
        assert!(!report.truncated);
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn test_scan_sensitive_file_encrypted_pdf_valid_credential() {
        let report = scan_sensitive_file(
            &fixture_path("encrypted.pdf"),
            Some("fixture-password"),
            &["\\b[0-9]+\\b".to_string()],
        )
        .expect("Encrypted PDF scan with valid credential should succeed");

        assert_eq!(report.schema_version, 2);
    }

    #[test]
    fn test_scan_sensitive_file_encrypted_pdf_invalid_or_missing_credential() {
        let res = scan_sensitive_file(
            &fixture_path("encrypted.pdf"),
            None,
            &["\\b[0-9]+\\b".to_string()],
        );

        assert!(res.is_err());
        if let SessionJobOperationError::Failed(f) = res.unwrap_err() {
            assert_eq!(f.code, "SESSION_JOB_PDF_OPEN_FAILED");
        } else {
            panic!("Expected SESSION_JOB_PDF_OPEN_FAILED");
        }
    }

    #[test]
    fn test_unresolved_page_geometry_decision_and_production_item_result() {
        let mut warnings = Vec::new();
        let fake_rect = PdfRect {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
        };

        // When page_geometry is None (extraction failed)
        let (status, geom, rects) = resolve_item_geometry_and_rects(
            GeometryStatus::Resolved,
            None,
            vec![fake_rect],
            &mut warnings,
        );

        assert_eq!(status, GeometryStatus::Unresolved);
        assert!(geom.is_none());
        assert!(rects.is_empty());
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "SENSITIVE_SCAN_PAGE_GEOMETRY_UNRESOLVED");

        // Production scan test with zero-width cropbox fixture
        let f_zero_crop = build_geometry_fixture_pdf(
            (0.0, 0.0, 400.0, 600.0),
            Some((100.0, 0.0, 100.0, 600.0)),
            0,
            "MATCH123",
        );
        let report =
            scan_sensitive_file(f_zero_crop.path(), None, &["MATCH\\d+".to_string()]).unwrap();
        assert_eq!(report.schema_version, 2);
        assert_eq!(report.items.len(), 1);
        let item = &report.items[0];
        assert_eq!(item.geometry_status, GeometryStatus::Unresolved);
        assert!(item.page_geometry.is_none());
        assert!(item.source_rects_pdf_points.is_empty());
        assert!(item
            .warnings
            .iter()
            .any(|w| w.code == "SENSITIVE_SCAN_PAGE_GEOMETRY_UNRESOLVED"));
    }
}
