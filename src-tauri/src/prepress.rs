use crate::document_session::SessionJobOperationError;
use crate::pdf_engine::{get_pdfium, open_pdfium_doc};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

pub const PRINT_INSPECTION_SCHEMA_VERSION: &str = "print-resource-inspection/1";
pub const PRINT_INSPECTION_VERSION: &str = "print-resource-inspection/1.0";
pub const RGB_PREVIEW_SCHEMA_VERSION: &str = "rgb-derived-print-preview/1";
pub const RGB_PREVIEW_METHODOLOGY_VERSION: &str = "naive-rgb-to-cmyk/1";

const MAX_INSPECTION_PAGES: usize = 10_000;
const MAX_INSPECTION_OBJECTS: usize = 100_000;
const MAX_TRAVERSAL_DEPTH: usize = 64;
const MAX_RESOURCE_REFERENCES: usize = 200_000;
const MAX_XOBJECTS: usize = 50_000;
const MAX_PATTERNS: usize = 20_000;
const MAX_SHADINGS: usize = 20_000;
const MAX_COLOR_SPACES: usize = 20_000;
const MAX_ICC_PROFILES: usize = 256;
const MAX_SINGLE_ICC_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOTAL_ICC_BYTES: usize = 64 * 1024 * 1024;
const MAX_OUTPUT_INTENTS: usize = 64;
const MAX_SPOT_COLORS: usize = 4_096;
const MAX_FONTS: usize = 20_000;
const MAX_TRANSPARENCY_OBSERVATIONS: usize = 20_000;
const MAX_FINDINGS: usize = 4_096;
const MAX_REPORT_BYTES: usize = 32 * 1024 * 1024;
const MAX_PREVIEW_DIMENSION: u32 = 16_384;
const MAX_PREVIEW_PIXELS: u64 = 24_000_000;
const MAX_PNG_BYTES: usize = 24 * 1024 * 1024;
const MAX_TOTAL_PREVIEW_BYTES: usize = 72 * 1024 * 1024;
const MIN_PREVIEW_SCALE: f32 = 0.1;
const MAX_PREVIEW_SCALE: f32 = 4.0;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepressResultIdentity {
    pub session_id: String,
    pub content_generation: u64,
    pub physical_page_index: u32,
    pub request_id: String,
    pub methodology_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentFacts {
    pub pdf_version: String,
    pub page_count: u32,
    pub encrypted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceCounts {
    pub visited_objects: u32,
    pub resource_references: u32,
    pub xobjects: u32,
    pub patterns: u32,
    pub shadings: u32,
    pub color_space_definitions: u32,
    pub unique_color_space_objects: u32,
    pub icc_profiles: u32,
    pub output_intents: u32,
    pub spot_colors: u32,
    pub fonts: u32,
    pub transparency_observations: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionArea {
    pub area: String,
    pub status: String,
    pub limitation: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionCoverage {
    pub inspected_areas: Vec<String>,
    pub not_inspected_areas: Vec<String>,
    pub incomplete_areas: Vec<String>,
    pub limitations: Vec<String>,
    pub resource_counts: ResourceCounts,
    pub pages_inspected: u32,
    pub areas: Vec<InspectionArea>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorSpaceObservation {
    pub id: String,
    pub object_id: Option<String>,
    pub resource_name: Option<String>,
    pub family: String,
    pub definition_count: u32,
    pub reference_count: u32,
    pub affected_pages: Vec<u32>,
    pub alternate_family: Option<String>,
    pub usage_status: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IccProfileObservation {
    pub object_id: Option<String>,
    pub inspection_level: String,
    pub stream_present: bool,
    pub encoded_length: u64,
    pub decoded_length: Option<u64>,
    pub sha256: Option<String>,
    pub component_count: Option<i64>,
    pub alternate_color_space: Option<String>,
    pub declared_size: Option<u32>,
    pub signature_acsp: Option<bool>,
    pub profile_version: Option<String>,
    pub device_class: Option<String>,
    pub input_color_space: Option<String>,
    pub pcs: Option<String>,
    pub status: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputIntentObservation {
    pub object_id: Option<String>,
    pub dictionary_present: bool,
    pub type_name: Option<String>,
    pub subtype: Option<String>,
    pub output_condition_identifier: Option<String>,
    pub registry_name: Option<String>,
    pub has_dest_output_profile: bool,
    pub has_external_profile_reference: bool,
    pub profile_observation_id: Option<String>,
    pub status: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotColorObservation {
    pub object_id: Option<String>,
    pub family: String,
    pub colorant_names: Vec<String>,
    pub contains_all: bool,
    pub contains_none: bool,
    pub alternate_family: Option<String>,
    pub tint_transform_present: bool,
    pub attributes_present: bool,
    pub affected_pages: Vec<u32>,
    pub usage_status: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransparencyObservation {
    pub object_id: Option<String>,
    pub kind: String,
    pub keys: Vec<String>,
    pub affected_pages: Vec<u32>,
    pub usage_status: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontObservation {
    pub object_id: Option<String>,
    pub resource_name: String,
    pub subtype: Option<String>,
    pub base_font: Option<String>,
    pub embedded_stream_present: bool,
    pub subset_name_indication: bool,
    pub affected_pages: Vec<u32>,
    pub usage_status: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageBoxObservation {
    pub coordinates: Option<[f32; 4]>,
    pub source: String,
    pub valid_geometry: bool,
    pub non_zero_origin: bool,
    pub negative_origin: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageBoxesObservation {
    pub physical_page_index: u32,
    pub rotation_degrees: i32,
    pub media_box: PageBoxObservation,
    pub crop_box: PageBoxObservation,
    pub bleed_box: PageBoxObservation,
    pub trim_box: PageBoxObservation,
    pub art_box: PageBoxObservation,
    pub containment_warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintResourceInspectionReport {
    pub schema_version: String,
    pub inspection_version: String,
    pub identity: PrepressResultIdentity,
    pub summary_status: String,
    pub document_facts: DocumentFacts,
    pub color_spaces: Vec<ColorSpaceObservation>,
    pub icc_profiles: Vec<IccProfileObservation>,
    pub output_intents: Vec<OutputIntentObservation>,
    pub spot_colors: Vec<SpotColorObservation>,
    pub transparency: Vec<TransparencyObservation>,
    pub fonts: Vec<FontObservation>,
    pub page_boxes: Vec<PageBoxesObservation>,
    pub coverage: InspectionCoverage,
    pub warnings: Vec<String>,
    pub truncated: bool,
    pub conformance_claim: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RgbDerivedPreviewMetadata {
    pub schema_version: String,
    pub methodology: String,
    pub source_color_model: String,
    pub approximate: bool,
    pub color_managed: bool,
    pub output_intent_applied: bool,
    pub spot_colors_preserved: bool,
    pub overprint_simulated: bool,
    pub rendering_intent: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoveragePixelStatistics {
    pub pixels_above_threshold: u64,
    pub total_pixels: u64,
    pub maximum_estimated_channel_sum_percent: f64,
    pub average_estimated_channel_sum_percent: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RgbDerivedCoveragePreview {
    pub identity: PrepressResultIdentity,
    pub metadata: RgbDerivedPreviewMetadata,
    pub page_index: u32,
    pub width: u32,
    pub height: u32,
    pub threshold_percent: f32,
    pub heatmap_png_base64: String,
    pub encoded_png_bytes: u64,
    pub pixel_statistics: CoveragePixelStatistics,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RgbDerivedProcessPreview {
    pub identity: PrepressResultIdentity,
    pub metadata: RgbDerivedPreviewMetadata,
    pub page_index: u32,
    pub width: u32,
    pub height: u32,
    pub c_preview_png_base64: String,
    pub m_preview_png_base64: String,
    pub y_preview_png_base64: String,
    pub k_preview_png_base64: String,
    pub encoded_png_bytes: [u64; 4],
    pub total_encoded_png_bytes: u64,
    pub truncated: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct PrepressCancellation<'a> {
    marker: Option<&'a Path>,
}

impl<'a> PrepressCancellation<'a> {
    #[cfg(test)]
    pub(crate) fn none() -> Self {
        Self { marker: None }
    }

    pub(crate) fn marker(path: &'a Path) -> Self {
        Self { marker: Some(path) }
    }

    pub(crate) fn check(self) -> Result<(), SessionJobOperationError> {
        if self.marker.is_some_and(Path::exists) {
            return Err(SessionJobOperationError::failed_with_code(
                "SESSION_JOB_WORKER_CANCELLED",
                "Print inspection veya preview worker işlemi iptal edildi",
            ));
        }
        Ok(())
    }
}

#[derive(Default)]
struct MutableColorSpace {
    object_id: Option<String>,
    resource_name: Option<String>,
    family: String,
    references: u32,
    definitions: u32,
    pages: BTreeSet<u32>,
    alternate: Option<String>,
}

struct InspectionScanner<'a> {
    document: &'a Document,
    cancellation: PrepressCancellation<'a>,
    visited_objects: HashSet<ObjectId>,
    resource_references: usize,
    xobjects_scanned: usize,
    patterns_scanned: usize,
    shadings_scanned: usize,
    pages_inspected: usize,
    color_spaces: BTreeMap<String, MutableColorSpace>,
    icc_profiles: Vec<IccProfileObservation>,
    icc_object_indices: BTreeMap<ObjectId, usize>,
    total_icc_bytes: usize,
    output_intents: Vec<OutputIntentObservation>,
    spot_colors: Vec<SpotColorObservation>,
    transparency: Vec<TransparencyObservation>,
    fonts: Vec<FontObservation>,
    page_boxes: Vec<PageBoxesObservation>,
    warnings: Vec<String>,
    truncated: bool,
}

impl<'a> InspectionScanner<'a> {
    fn new(document: &'a Document, cancellation: PrepressCancellation<'a>) -> Self {
        Self {
            document,
            cancellation,
            visited_objects: HashSet::new(),
            resource_references: 0,
            xobjects_scanned: 0,
            patterns_scanned: 0,
            shadings_scanned: 0,
            pages_inspected: 0,
            color_spaces: BTreeMap::new(),
            icc_profiles: Vec::new(),
            icc_object_indices: BTreeMap::new(),
            total_icc_bytes: 0,
            output_intents: Vec::new(),
            spot_colors: Vec::new(),
            transparency: Vec::new(),
            fonts: Vec::new(),
            page_boxes: Vec::new(),
            warnings: Vec::new(),
            truncated: false,
        }
    }

    fn warning(&mut self, message: impl Into<String>) {
        if self.warnings.len() < MAX_FINDINGS {
            self.warnings.push(message.into());
        } else {
            self.truncated = true;
        }
    }

    fn push_transparency(&mut self, observation: TransparencyObservation) {
        if self.transparency.len() < MAX_TRANSPARENCY_OBSERVATIONS {
            self.transparency.push(observation);
        } else {
            self.truncated = true;
            self.warning("Transparency/overprint observation limit exceeded");
        }
    }

    fn touch_reference(&mut self, object_id: Option<ObjectId>) -> bool {
        self.resource_references = self.resource_references.saturating_add(1);
        if self.resource_references > MAX_RESOURCE_REFERENCES {
            self.truncated = true;
            return false;
        }
        if let Some(object_id) = object_id {
            self.visited_objects.insert(object_id);
            if self.visited_objects.len() > MAX_INSPECTION_OBJECTS {
                self.truncated = true;
                return false;
            }
        }
        true
    }

    fn resolve_object(&self, object: &Object) -> Result<(Object, Option<ObjectId>), String> {
        let mut current = object;
        let mut last_id = None;
        let mut seen = HashSet::new();
        for _ in 0..=MAX_TRAVERSAL_DEPTH {
            let Object::Reference(object_id) = current else {
                return Ok((current.clone(), last_id));
            };
            if !seen.insert(*object_id) {
                return Err(format!(
                    "Indirect reference cycle at {} {} R",
                    object_id.0, object_id.1
                ));
            }
            last_id = Some(*object_id);
            current =
                self.document.objects.get(object_id).ok_or_else(|| {
                    format!("Dangling reference {} {} R", object_id.0, object_id.1)
                })?;
        }
        Err("Indirect reference depth limit exceeded".to_string())
    }

    fn scan(mut self) -> Result<Self, SessionJobOperationError> {
        self.cancellation.check()?;
        self.scan_output_intents();
        let pages = self.document.get_pages();
        for (ordinal, page_id) in pages.values().copied().enumerate() {
            self.cancellation.check()?;
            if ordinal >= MAX_INSPECTION_PAGES {
                self.truncated = true;
                self.warning("Page inspection limit exceeded");
                break;
            }
            let page_index = u32::try_from(ordinal).unwrap_or(u32::MAX);
            self.pages_inspected += 1;
            self.scan_page_boxes(page_id, page_index);
            match self.find_inherited(page_id, b"Resources") {
                Ok(Some((resources, source))) => match self.resolve_object(resources) {
                    Ok((Object::Dictionary(dictionary), object_id)) => {
                        self.scan_resources(
                            &dictionary,
                            object_id,
                            page_index,
                            &source,
                            0,
                            &mut HashSet::new(),
                        );
                    }
                    Ok((_, _)) => {
                        self.warning(format!("Page {page_index} Resources is not a dictionary"))
                    }
                    Err(error) => self.warning(format!("Page {page_index} Resources: {error}")),
                },
                Ok(None) => {}
                Err(error) => {
                    self.warning(format!("Page {page_index} inherited resources: {error}"))
                }
            }
        }
        Ok(self)
    }

    fn find_inherited<'b>(
        &'b self,
        page_id: ObjectId,
        key: &[u8],
    ) -> Result<Option<(&'b Object, String)>, String> {
        let mut current = page_id;
        let mut seen = HashSet::new();
        for depth in 0..=MAX_TRAVERSAL_DEPTH {
            if !seen.insert(current) {
                return Err("page-tree inheritance cycle".to_string());
            }
            let dictionary = self
                .document
                .get_dictionary(current)
                .map_err(|error| format!("page-tree dictionary unavailable: {error}"))?;
            if let Ok(value) = dictionary.get(key) {
                let source = if depth == 0 { "direct" } else { "inherited" };
                return Ok(Some((value, source.to_string())));
            }
            let Ok(Object::Reference(parent)) = dictionary.get(b"Parent") else {
                return Ok(None);
            };
            current = *parent;
        }
        Err("page-tree inheritance depth limit exceeded".to_string())
    }

    fn scan_resources(
        &mut self,
        dictionary: &Dictionary,
        object_id: Option<ObjectId>,
        page_index: u32,
        path: &str,
        depth: usize,
        stack: &mut HashSet<ObjectId>,
    ) {
        if depth > MAX_TRAVERSAL_DEPTH || !self.touch_reference(object_id) {
            self.truncated = true;
            return;
        }
        if let Some(id) = object_id {
            if !stack.insert(id) {
                self.warning(format!("Resource cycle at {} {} R", id.0, id.1));
                return;
            }
        }

        if let Some(color_spaces) = self.resolved_dictionary_entry(dictionary, b"ColorSpace", path)
        {
            for (name, object) in color_spaces.iter() {
                self.scan_color_space(
                    object,
                    Some(pdf_name(name)),
                    page_index,
                    &format!("{path}/ColorSpace/{}", pdf_name(name)),
                    depth + 1,
                );
            }
        }
        if let Some(ext_g_states) = self.resolved_dictionary_entry(dictionary, b"ExtGState", path) {
            for (_, object) in ext_g_states.iter() {
                self.scan_ext_g_state(object, page_index);
            }
        }
        if let Some(fonts) = self.resolved_dictionary_entry(dictionary, b"Font", path) {
            for (name, object) in fonts.iter() {
                self.scan_font(object, pdf_name(name), page_index);
            }
        }
        if let Some(x_objects) = self.resolved_dictionary_entry(dictionary, b"XObject", path) {
            for (name, object) in x_objects.iter() {
                if self.xobjects_scanned >= MAX_XOBJECTS {
                    self.truncated = true;
                    self.warning("XObject inspection limit exceeded");
                    break;
                }
                self.xobjects_scanned += 1;
                self.scan_xobject(
                    object,
                    page_index,
                    &format!("{path}/XObject/{}", pdf_name(name)),
                    depth + 1,
                    stack,
                );
            }
        }
        if let Some(patterns) = self.resolved_dictionary_entry(dictionary, b"Pattern", path) {
            for (name, object) in patterns.iter() {
                if self.patterns_scanned >= MAX_PATTERNS {
                    self.truncated = true;
                    self.warning("Pattern inspection limit exceeded");
                    break;
                }
                self.patterns_scanned += 1;
                self.scan_pattern(
                    object,
                    page_index,
                    &format!("{path}/Pattern/{}", pdf_name(name)),
                    depth + 1,
                    stack,
                );
            }
        }
        if let Some(shadings) = self.resolved_dictionary_entry(dictionary, b"Shading", path) {
            for (name, object) in shadings.iter() {
                if self.shadings_scanned >= MAX_SHADINGS {
                    self.truncated = true;
                    self.warning("Shading inspection limit exceeded");
                    break;
                }
                self.shadings_scanned += 1;
                self.scan_shading(
                    object,
                    page_index,
                    &format!("{path}/Shading/{}", pdf_name(name)),
                    depth + 1,
                );
            }
        }

        if let Some(id) = object_id {
            stack.remove(&id);
        }
    }

    fn resolved_dictionary_entry(
        &mut self,
        dictionary: &Dictionary,
        key: &[u8],
        path: &str,
    ) -> Option<Dictionary> {
        let object = dictionary.get(key).ok()?;
        match self.resolve_object(object) {
            Ok((Object::Dictionary(value), object_id)) => {
                self.touch_reference(object_id);
                Some(value)
            }
            Ok((_, _)) => {
                self.warning(format!("{path}/{} is not a dictionary", pdf_name(key)));
                None
            }
            Err(error) => {
                self.warning(format!("{path}/{}: {error}", pdf_name(key)));
                None
            }
        }
    }

    fn scan_color_space(
        &mut self,
        object: &Object,
        resource_name: Option<String>,
        page_index: u32,
        path: &str,
        depth: usize,
    ) {
        if self.color_spaces.len() >= MAX_COLOR_SPACES || depth > MAX_TRAVERSAL_DEPTH {
            self.truncated = true;
            return;
        }
        let (resolved, object_id) = match self.resolve_object(object) {
            Ok(value) => value,
            Err(error) => {
                self.warning(format!("{path}: {error}"));
                return;
            }
        };
        if !self.touch_reference(object_id) {
            return;
        }
        let (family, array) = color_space_family(&resolved);
        let key = object_id
            .map(object_id_string)
            .unwrap_or_else(|| format!("{path}:{family}"));
        let alternate = array
            .and_then(|items| items.get(2))
            .map(|item| color_space_family(item).0);
        let entry = self
            .color_spaces
            .entry(key.clone())
            .or_insert_with(|| MutableColorSpace {
                object_id: object_id.map(object_id_string),
                resource_name: resource_name.clone(),
                family: family.clone(),
                references: 0,
                definitions: 0,
                pages: BTreeSet::new(),
                alternate: alternate.clone(),
            });
        entry.references = entry.references.saturating_add(1);
        entry.definitions = entry.definitions.saturating_add(1);
        entry.pages.insert(page_index);

        if let Some(items) = array {
            if family == "icc_based" {
                self.scan_icc(items.get(1), object_id);
            }
            if family == "separation" || family == "device_n" || family == "n_channel" {
                self.scan_spot(items, object_id, family.clone(), page_index);
            }
            if matches!(
                family.as_str(),
                "icc_based" | "indexed" | "separation" | "device_n" | "n_channel"
            ) {
                if let Some(alternate_object) = items.get(2) {
                    self.scan_color_space(
                        alternate_object,
                        None,
                        page_index,
                        &format!("{path}/alternate"),
                        depth + 1,
                    );
                }
            }
            if family == "pattern" {
                if let Some(base) = items.get(1) {
                    self.scan_color_space(
                        base,
                        None,
                        page_index,
                        &format!("{path}/base"),
                        depth + 1,
                    );
                }
            }
        }
    }

    fn scan_icc(
        &mut self,
        stream_object: Option<&Object>,
        color_space_id: Option<ObjectId>,
    ) -> Option<String> {
        if self.icc_profiles.len() >= MAX_ICC_PROFILES {
            self.truncated = true;
            return None;
        }
        let Some(stream_object) = stream_object else {
            self.warning("ICCBased definition has no stream");
            return None;
        };
        let (resolved, stream_id) = match self.resolve_object(stream_object) {
            Ok(value) => value,
            Err(error) => {
                self.warning(format!("ICCBased stream: {error}"));
                return None;
            }
        };
        if let Some(stream_id) = stream_id {
            if let Some(index) = self.icc_object_indices.get(&stream_id).copied() {
                return Some(format!("icc-profile-{index}"));
            }
        }
        let Object::Stream(stream) = resolved else {
            self.warning("ICCBased profile object is not a stream");
            return None;
        };
        let mut observation = inspect_icc_stream(&stream, stream_id, self.total_icc_bytes);
        if observation.decoded_length.is_some_and(|length| {
            self.total_icc_bytes.saturating_add(length as usize) > MAX_TOTAL_ICC_BYTES
        }) {
            observation.status = "limit_exceeded".to_string();
            observation.sha256 = None;
            observation
                .warnings
                .push("Total decoded ICC byte limit exceeded".to_string());
            self.truncated = true;
        } else {
            self.total_icc_bytes = self
                .total_icc_bytes
                .saturating_add(observation.decoded_length.unwrap_or(0) as usize);
        }
        if observation.object_id.is_none() {
            observation.object_id = color_space_id.map(object_id_string);
        }
        let index = self.icc_profiles.len();
        if let Some(stream_id) = stream_id {
            self.icc_object_indices.insert(stream_id, index);
        }
        self.icc_profiles.push(observation);
        Some(format!("icc-profile-{index}"))
    }

    fn scan_spot(
        &mut self,
        items: &[Object],
        object_id: Option<ObjectId>,
        family: String,
        page_index: u32,
    ) {
        if self.spot_colors.len() >= MAX_SPOT_COLORS {
            self.truncated = true;
            return;
        }
        let names = if family == "separation" {
            items
                .get(1)
                .and_then(object_name)
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            items
                .get(1)
                .and_then(|object| object.as_array().ok())
                .map(|values| values.iter().filter_map(object_name).collect())
                .unwrap_or_default()
        };
        let mut warnings = Vec::new();
        if names.is_empty() {
            warnings.push("Colorant name is missing or malformed".to_string());
        }
        let alternate = items.get(2).map(|value| color_space_family(value).0);
        let tint_present = items
            .get(3)
            .is_some_and(|value| !matches!(value, Object::Null));
        if alternate.is_none() {
            warnings.push("Alternate color space is missing".to_string());
        }
        if !tint_present {
            warnings.push("Tint transform is missing".to_string());
        }
        if names.iter().any(|name| {
            self.spot_colors
                .iter()
                .any(|existing| existing.colorant_names.contains(name))
        }) {
            warnings.push(
                "Duplicate/conflicting colorant name observed; usage and tint behavior were not validated"
                    .to_string(),
            );
        }
        self.spot_colors.push(SpotColorObservation {
            object_id: object_id.map(object_id_string),
            family,
            contains_all: names.iter().any(|name| name == "All"),
            contains_none: names.iter().any(|name| name == "None"),
            colorant_names: names,
            alternate_family: alternate,
            tint_transform_present: tint_present,
            attributes_present: items
                .get(4)
                .is_some_and(|value| !matches!(value, Object::Null)),
            affected_pages: vec![page_index],
            usage_status: "usage_not_checked".to_string(),
            warnings,
        });
    }

    fn scan_xobject(
        &mut self,
        object: &Object,
        page_index: u32,
        path: &str,
        depth: usize,
        stack: &mut HashSet<ObjectId>,
    ) {
        let (resolved, object_id) = match self.resolve_object(object) {
            Ok(value) => value,
            Err(error) => {
                self.warning(format!("{path}: {error}"));
                return;
            }
        };
        self.touch_reference(object_id);
        let Object::Stream(stream) = resolved else {
            self.warning(format!("{path} is not a stream"));
            return;
        };
        let subtype = stream
            .dict
            .get(b"Subtype")
            .ok()
            .and_then(object_name)
            .unwrap_or_default();
        if subtype == "Form" {
            if let Ok(resources) = stream.dict.get(b"Resources") {
                match self.resolve_object(resources) {
                    Ok((Object::Dictionary(dictionary), resource_id)) => self.scan_resources(
                        &dictionary,
                        resource_id,
                        page_index,
                        path,
                        depth,
                        stack,
                    ),
                    Ok((_, _)) => self.warning(format!("{path}/Resources is not a dictionary")),
                    Err(error) => self.warning(format!("{path}/Resources: {error}")),
                }
            }
            if let Ok(group) = stream.dict.get(b"Group") {
                let is_transparency = self
                    .resolve_object(group)
                    .ok()
                    .and_then(|(value, _)| {
                        value
                            .as_dict()
                            .ok()
                            .and_then(|dict| dict.get(b"S").ok())
                            .and_then(object_name)
                    })
                    .is_some_and(|name| name == "Transparency");
                if is_transparency {
                    self.push_transparency(TransparencyObservation {
                        object_id: object_id.map(object_id_string),
                        kind: "transparency_group".to_string(),
                        keys: vec!["Group/S/Transparency".to_string()],
                        affected_pages: vec![page_index],
                        usage_status: "usage_not_checked".to_string(),
                    });
                }
            }
        } else if subtype == "Image" {
            if let Ok(color_space) = stream.dict.get(b"ColorSpace") {
                self.scan_color_space(
                    color_space,
                    None,
                    page_index,
                    &format!("{path}/ColorSpace"),
                    depth,
                );
            }
            let keys = [b"SMask".as_slice(), b"Mask".as_slice()]
                .into_iter()
                .filter(|key| stream.dict.has(key))
                .map(pdf_name)
                .collect::<Vec<_>>();
            if !keys.is_empty() {
                self.push_transparency(TransparencyObservation {
                    object_id: object_id.map(object_id_string),
                    kind: "image_mask".to_string(),
                    keys,
                    affected_pages: vec![page_index],
                    usage_status: "usage_not_checked".to_string(),
                });
            }
        }
    }

    fn scan_pattern(
        &mut self,
        object: &Object,
        page_index: u32,
        path: &str,
        depth: usize,
        stack: &mut HashSet<ObjectId>,
    ) {
        let (resolved, object_id) = match self.resolve_object(object) {
            Ok(value) => value,
            Err(error) => {
                self.warning(format!("{path}: {error}"));
                return;
            }
        };
        let dictionary = match resolved {
            Object::Dictionary(value) => value,
            Object::Stream(value) => value.dict,
            _ => {
                self.warning(format!("{path} is not a pattern dictionary"));
                return;
            }
        };
        if let Ok(resources) = dictionary.get(b"Resources") {
            if let Ok((Object::Dictionary(resources), resource_id)) = self.resolve_object(resources)
            {
                self.scan_resources(
                    &resources,
                    resource_id.or(object_id),
                    page_index,
                    path,
                    depth,
                    stack,
                );
            }
        }
    }

    fn scan_shading(&mut self, object: &Object, page_index: u32, path: &str, depth: usize) {
        let (resolved, _) = match self.resolve_object(object) {
            Ok(value) => value,
            Err(error) => {
                self.warning(format!("{path}: {error}"));
                return;
            }
        };
        let dictionary = match resolved {
            Object::Dictionary(value) => value,
            Object::Stream(value) => value.dict,
            _ => return,
        };
        if let Ok(color_space) = dictionary.get(b"ColorSpace") {
            self.scan_color_space(
                color_space,
                None,
                page_index,
                &format!("{path}/ColorSpace"),
                depth,
            );
        }
    }

    fn scan_ext_g_state(&mut self, object: &Object, page_index: u32) {
        let (resolved, object_id) = match self.resolve_object(object) {
            Ok(value) => value,
            Err(_) => return,
        };
        let Ok(dictionary) = resolved.as_dict() else {
            return;
        };
        let keys = [
            b"CA".as_slice(),
            b"ca".as_slice(),
            b"BM".as_slice(),
            b"SMask".as_slice(),
            b"OP".as_slice(),
            b"op".as_slice(),
            b"OPM".as_slice(),
        ]
        .into_iter()
        .filter(|key| dictionary.has(key))
        .map(pdf_name)
        .collect::<Vec<_>>();
        if !keys.is_empty() {
            self.push_transparency(TransparencyObservation {
                object_id: object_id.map(object_id_string),
                kind: "ext_g_state".to_string(),
                keys,
                affected_pages: vec![page_index],
                usage_status: "usage_not_checked".to_string(),
            });
        }
    }

    fn scan_font(&mut self, object: &Object, resource_name: String, page_index: u32) {
        if self.fonts.len() >= MAX_FONTS {
            self.truncated = true;
            return;
        }
        let (resolved, object_id) = match self.resolve_object(object) {
            Ok(value) => value,
            Err(error) => {
                self.warning(format!("Font {resource_name}: {error}"));
                return;
            }
        };
        let Ok(dictionary) = resolved.as_dict() else {
            return;
        };
        let subtype = dictionary.get(b"Subtype").ok().and_then(object_name);
        let base_font = dictionary.get(b"BaseFont").ok().and_then(object_name);
        let embedded = dictionary
            .get(b"FontDescriptor")
            .ok()
            .and_then(|value| self.resolve_object(value).ok())
            .and_then(|(value, _)| {
                value.as_dict().ok().map(|descriptor| {
                    descriptor.has(b"FontFile")
                        || descriptor.has(b"FontFile2")
                        || descriptor.has(b"FontFile3")
                })
            })
            .unwrap_or(false);
        let subset = base_font.as_deref().is_some_and(is_subset_name);
        self.fonts.push(FontObservation { object_id: object_id.map(object_id_string), resource_name, subtype, base_font, embedded_stream_present: embedded, subset_name_indication: subset, affected_pages: vec![page_index], usage_status: "usage_not_checked".to_string(), warnings: vec!["Resource definition observed; glyph usage and embedding completeness were not checked".to_string()] });
    }

    fn scan_output_intents(&mut self) {
        let Ok(catalog) = self.document.catalog() else {
            self.warning("Catalog dictionary unavailable");
            return;
        };
        let Ok(output_intents) = catalog.get(b"OutputIntents") else {
            return;
        };
        let (resolved, _) = match self.resolve_object(output_intents) {
            Ok(value) => value,
            Err(error) => {
                self.warning(format!("OutputIntents: {error}"));
                return;
            }
        };
        let Object::Array(values) = resolved else {
            self.warning("OutputIntents is not an array");
            return;
        };
        for value in values.iter().take(MAX_OUTPUT_INTENTS) {
            let (resolved, object_id) = match self.resolve_object(value) {
                Ok(value) => value,
                Err(error) => {
                    self.warning(format!("OutputIntent: {error}"));
                    continue;
                }
            };
            let Ok(dictionary) = resolved.as_dict() else {
                self.warning("OutputIntent array member is not a dictionary");
                continue;
            };
            let mut warnings = vec![
                "Dictionary observed; profile and PDF/X policy validation were not performed"
                    .to_string(),
            ];
            let type_name = dictionary.get(b"Type").ok().and_then(object_name);
            if type_name.as_deref() != Some("OutputIntent") {
                warnings.push("Type is not OutputIntent".to_string());
            }
            let has_profile = dictionary.has(b"DestOutputProfile");
            let profile_id = dictionary
                .get(b"DestOutputProfile")
                .ok()
                .and_then(|profile| self.scan_icc(Some(profile), None));
            self.output_intents.push(OutputIntentObservation {
                object_id: object_id.map(object_id_string),
                dictionary_present: true,
                type_name,
                subtype: dictionary.get(b"S").ok().and_then(object_name),
                output_condition_identifier: dictionary
                    .get(b"OutputConditionIdentifier")
                    .ok()
                    .and_then(object_text),
                registry_name: dictionary.get(b"RegistryName").ok().and_then(object_text),
                has_dest_output_profile: has_profile,
                has_external_profile_reference: dictionary.has(b"DestOutputProfileRef"),
                profile_observation_id: profile_id,
                status: "observed_not_validated".to_string(),
                warnings,
            });
        }
        if values.len() > MAX_OUTPUT_INTENTS {
            self.truncated = true;
            self.warning("OutputIntent limit exceeded");
        }
        if values.len() > 1 {
            self.warning(
                "Multiple OutputIntent dictionaries were found; conflict policy was not validated",
            );
        }
    }

    fn scan_page_boxes(&mut self, page_id: ObjectId, page_index: u32) {
        let media = self.structural_box(page_id, b"MediaBox", None);
        let crop = self.structural_box(page_id, b"CropBox", media.coordinates);
        let bleed = self.structural_box(page_id, b"BleedBox", crop.coordinates);
        let trim = self.structural_box(page_id, b"TrimBox", crop.coordinates);
        let art = self.structural_box(page_id, b"ArtBox", crop.coordinates);
        let rotation = self
            .find_inherited(page_id, b"Rotate")
            .ok()
            .flatten()
            .and_then(|(value, _)| value.as_i64().ok())
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(0);
        let mut containment_warnings = Vec::new();
        if let Some(media_coords) = media.coordinates {
            for (name, observation) in [
                ("CropBox", &crop),
                ("BleedBox", &bleed),
                ("TrimBox", &trim),
                ("ArtBox", &art),
            ] {
                if observation
                    .coordinates
                    .is_some_and(|coords| !contains_box(media_coords, coords))
                {
                    containment_warnings.push(format!(
                        "{name} is not contained by MediaBox; no PDF/X violation claim is made"
                    ));
                }
            }
        }
        self.page_boxes.push(PageBoxesObservation {
            physical_page_index: page_index,
            rotation_degrees: rotation,
            media_box: media,
            crop_box: crop,
            bleed_box: bleed,
            trim_box: trim,
            art_box: art,
            containment_warnings,
        });
    }

    fn structural_box(
        &mut self,
        page_id: ObjectId,
        key: &[u8],
        fallback: Option<[f32; 4]>,
    ) -> PageBoxObservation {
        match self.find_inherited(page_id, key) {
            Ok(Some((object, source))) => match parse_box(object) {
                Some(coords) => box_observation(Some(coords), source),
                None => {
                    self.warning(format!("{} has malformed coordinates", pdf_name(key)));
                    box_observation(None, "malformed".to_string())
                }
            },
            Ok(None) if fallback.is_some() => box_observation(fallback, "defaulted".to_string()),
            Ok(None) => box_observation(None, "missing".to_string()),
            Err(error) => {
                self.warning(format!("{} inheritance: {error}", pdf_name(key)));
                box_observation(None, "incomplete".to_string())
            }
        }
    }
}

pub(crate) fn inspect_print_resources(
    file_path: String,
    password: Option<String>,
    identity: PrepressResultIdentity,
    cancellation: PrepressCancellation<'_>,
) -> Result<PrintResourceInspectionReport, SessionJobOperationError> {
    cancellation.check()?;
    if password.is_some() {
        return Err(SessionJobOperationError::failed_with_code(
            "PRINT_INSPECTION_ENCRYPTED_INPUT_UNSUPPORTED",
            "Print resource inspection encrypted session snapshot girdisini desteklemiyor",
        ));
    }
    let document = Document::load(&file_path).map_err(|error| {
        SessionJobOperationError::failed_with_code(
            "PRINT_INSPECTION_PARSE_FAILED",
            format!("Print resource inspection PDF snapshot'ını okuyamadı: {error}"),
        )
    })?;
    let encrypted = crate::lopdf_compat::was_encrypted(&document);
    let scanner = InspectionScanner::new(&document, cancellation).scan()?;
    let mut page_boxes = scanner.page_boxes;
    apply_pdfium_page_geometry(&file_path, &mut page_boxes, cancellation)?;
    let color_spaces = scanner
        .color_spaces
        .into_iter()
        .map(|(id, value)| ColorSpaceObservation {
            id,
            object_id: value.object_id,
            resource_name: value.resource_name,
            family: value.family,
            definition_count: value.definitions,
            reference_count: value.references,
            affected_pages: value.pages.into_iter().collect(),
            alternate_family: value.alternate,
            usage_status: "usage_not_checked".to_string(),
        })
        .collect::<Vec<_>>();
    let unique_color_space_objects = color_spaces
        .iter()
        .filter_map(|item| item.object_id.as_deref())
        .collect::<BTreeSet<_>>()
        .len();
    let inspected_areas = vec![
        "catalog",
        "page_dictionaries",
        "inherited_resources",
        "form_xobjects",
        "image_xobjects",
        "patterns",
        "shadings",
        "color_space",
        "output_intents",
        "font_resources",
        "ext_g_state",
        "page_boxes",
    ];
    let not_inspected_areas = vec![
        "content_stream_usage",
        "inline_images",
        "annotations",
        "optional_content",
        "external_references",
    ];
    let mut incomplete_areas = Vec::new();
    if scanner.truncated {
        incomplete_areas.push("resource_graph");
    }
    let limitations = vec![
        "Resource definitions were inspected; visible content-stream usage was not resolved"
            .to_string(),
        "ICC inspection is limited to a bounded basic header observation".to_string(),
        "No PDF/X conformance, print readiness, plate, RIP, or real ink claim is produced"
            .to_string(),
    ];
    let resource_counts = ResourceCounts {
        visited_objects: scanner.visited_objects.len().min(u32::MAX as usize) as u32,
        resource_references: scanner.resource_references.min(u32::MAX as usize) as u32,
        xobjects: scanner.xobjects_scanned.min(u32::MAX as usize) as u32,
        patterns: scanner.patterns_scanned.min(u32::MAX as usize) as u32,
        shadings: scanner.shadings_scanned.min(u32::MAX as usize) as u32,
        color_space_definitions: color_spaces
            .iter()
            .map(|item| u64::from(item.definition_count))
            .sum::<u64>()
            .min(u64::from(u32::MAX)) as u32,
        unique_color_space_objects: unique_color_space_objects.min(u32::MAX as usize) as u32,
        icc_profiles: scanner.icc_profiles.len().min(u32::MAX as usize) as u32,
        output_intents: scanner.output_intents.len().min(u32::MAX as usize) as u32,
        spot_colors: scanner.spot_colors.len().min(u32::MAX as usize) as u32,
        fonts: scanner.fonts.len().min(u32::MAX as usize) as u32,
        transparency_observations: scanner.transparency.len().min(u32::MAX as usize) as u32,
    };
    let mut areas = Vec::new();
    for area in &inspected_areas {
        areas.push(InspectionArea {
            area: (*area).to_string(),
            status: "inspected".to_string(),
            limitation: None,
        });
    }
    for area in &not_inspected_areas {
        areas.push(InspectionArea {
            area: (*area).to_string(),
            status: "not_inspected".to_string(),
            limitation: Some("Usage-level inspection is not implemented".to_string()),
        });
    }
    areas.push(InspectionArea {
        area: "encryption".to_string(),
        status: if encrypted { "incomplete" } else { "inspected" }.to_string(),
        limitation: if encrypted {
            Some("Encrypted structural inspection is unsupported".to_string())
        } else {
            None
        },
    });
    let has_observations = !color_spaces.is_empty()
        || !scanner.output_intents.is_empty()
        || !scanner.spot_colors.is_empty()
        || !scanner.transparency.is_empty()
        || !scanner.fonts.is_empty();
    let summary_status = if scanner.truncated || !not_inspected_areas.is_empty() {
        "inspection_incomplete"
    } else if has_observations {
        "findings_detected"
    } else {
        "no_findings_in_implemented_checks"
    };
    let report = PrintResourceInspectionReport {
        schema_version: PRINT_INSPECTION_SCHEMA_VERSION.to_string(),
        inspection_version: PRINT_INSPECTION_VERSION.to_string(),
        identity,
        summary_status: summary_status.to_string(),
        document_facts: DocumentFacts {
            pdf_version: document.version.clone(),
            page_count: document.get_pages().len().min(u32::MAX as usize) as u32,
            encrypted,
        },
        color_spaces,
        icc_profiles: scanner.icc_profiles,
        output_intents: scanner.output_intents,
        spot_colors: scanner.spot_colors,
        transparency: scanner.transparency,
        fonts: scanner.fonts,
        page_boxes,
        coverage: InspectionCoverage {
            inspected_areas: inspected_areas.into_iter().map(str::to_string).collect(),
            not_inspected_areas: not_inspected_areas
                .into_iter()
                .map(str::to_string)
                .collect(),
            incomplete_areas: incomplete_areas.into_iter().map(str::to_string).collect(),
            limitations,
            resource_counts,
            pages_inspected: scanner.pages_inspected.min(u32::MAX as usize) as u32,
            areas,
        },
        warnings: scanner.warnings,
        truncated: scanner.truncated,
        conformance_claim: false,
    };
    let report_bytes = serde_json::to_vec(&report).map_err(|error| {
        SessionJobOperationError::failed_with_code(
            "PRINT_INSPECTION_SERIALIZATION_FAILED",
            format!("Print resource inspection report serialize edilemedi: {error}"),
        )
    })?;
    if report_bytes.len() > MAX_REPORT_BYTES {
        return Err(SessionJobOperationError::failed_with_code(
            "PRINT_INSPECTION_REPORT_LIMIT_EXCEEDED",
            "Print resource inspection report byte limit exceeded",
        ));
    }
    cancellation.check()?;
    Ok(report)
}

pub(crate) fn generate_rgb_derived_coverage_preview(
    file_path: String,
    password: Option<String>,
    identity: PrepressResultIdentity,
    threshold_percent: f32,
    scale: f32,
    cancellation: PrepressCancellation<'_>,
) -> Result<RgbDerivedCoveragePreview, SessionJobOperationError> {
    if !threshold_percent.is_finite() || !(0.0..=400.0).contains(&threshold_percent) {
        return Err(preview_error(
            "RGB_PREVIEW_INVALID_THRESHOLD",
            "Threshold must be finite and between 0 and 400 percent",
        ));
    }
    let page_index = identity.physical_page_index;
    let rgba = render_preview_rgba(
        &file_path,
        password.as_deref(),
        page_index,
        scale,
        cancellation,
    )?;
    let (width, height) = rgba.dimensions();
    let mut heatmap = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::new(width, height);
    let mut above = 0_u64;
    let mut maximum = 0.0_f64;
    let mut sum = 0.0_f64;
    for (row, y) in (0..height).enumerate() {
        if row % 16 == 0 {
            cancellation.check()?;
        }
        for x in 0..width {
            let channels = rgb_to_estimated_process(rgba.get_pixel(x, y).0);
            let percent = f64::from((channels.iter().sum::<f32>() * 100.0).clamp(0.0, 400.0));
            maximum = maximum.max(percent);
            sum += percent;
            if percent > f64::from(threshold_percent) {
                above += 1;
                heatmap.put_pixel(x, y, image::Rgba([255, 0, 0, 180]));
            } else {
                heatmap.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            }
        }
    }
    cancellation.check()?;
    let png = encode_png(&heatmap, cancellation)?;
    let total_pixels = u64::from(width) * u64::from(height);
    let average = if total_pixels == 0 {
        0.0
    } else {
        sum / total_pixels as f64
    };
    Ok(RgbDerivedCoveragePreview {
        identity,
        metadata: preview_metadata(),
        page_index,
        width,
        height,
        threshold_percent,
        heatmap_png_base64: STANDARD.encode(&png),
        encoded_png_bytes: png.len() as u64,
        pixel_statistics: CoveragePixelStatistics {
            pixels_above_threshold: above,
            total_pixels,
            maximum_estimated_channel_sum_percent: maximum,
            average_estimated_channel_sum_percent: average,
        },
        truncated: false,
    })
}

pub(crate) fn generate_rgb_derived_process_preview(
    file_path: String,
    password: Option<String>,
    identity: PrepressResultIdentity,
    scale: f32,
    cancellation: PrepressCancellation<'_>,
) -> Result<RgbDerivedProcessPreview, SessionJobOperationError> {
    let page_index = identity.physical_page_index;
    let rgba = render_preview_rgba(
        &file_path,
        password.as_deref(),
        page_index,
        scale,
        cancellation,
    )?;
    let (width, height) = rgba.dimensions();
    let mut images = [
        image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::new(width, height),
        image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::new(width, height),
        image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::new(width, height),
        image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::new(width, height),
    ];
    for (row, y) in (0..height).enumerate() {
        if row % 16 == 0 {
            cancellation.check()?;
        }
        for x in 0..width {
            let values = rgb_to_estimated_process(rgba.get_pixel(x, y).0);
            let intensity = values.map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8);
            images[0].put_pixel(x, y, image::Rgba([255 - intensity[0], 255, 255, 255]));
            images[1].put_pixel(x, y, image::Rgba([255, 255 - intensity[1], 255, 255]));
            images[2].put_pixel(x, y, image::Rgba([255, 255, 255 - intensity[2], 255]));
            images[3].put_pixel(
                x,
                y,
                image::Rgba([
                    255 - intensity[3],
                    255 - intensity[3],
                    255 - intensity[3],
                    255,
                ]),
            );
        }
    }
    let mut encoded = Vec::with_capacity(4);
    for image in &images {
        cancellation.check()?;
        encoded.push(encode_png(image, cancellation)?);
    }
    let total = encoded.iter().map(Vec::len).sum::<usize>();
    if total > MAX_TOTAL_PREVIEW_BYTES {
        return Err(preview_error(
            "RGB_PREVIEW_RESULT_LIMIT_EXCEEDED",
            "Combined process preview PNG byte limit exceeded",
        ));
    }
    cancellation.check()?;
    let sizes = [
        encoded[0].len() as u64,
        encoded[1].len() as u64,
        encoded[2].len() as u64,
        encoded[3].len() as u64,
    ];
    Ok(RgbDerivedProcessPreview {
        identity,
        metadata: preview_metadata(),
        page_index,
        width,
        height,
        c_preview_png_base64: STANDARD.encode(&encoded[0]),
        m_preview_png_base64: STANDARD.encode(&encoded[1]),
        y_preview_png_base64: STANDARD.encode(&encoded[2]),
        k_preview_png_base64: STANDARD.encode(&encoded[3]),
        encoded_png_bytes: sizes,
        total_encoded_png_bytes: total as u64,
        truncated: false,
    })
}

fn preview_metadata() -> RgbDerivedPreviewMetadata {
    RgbDerivedPreviewMetadata {
        schema_version: RGB_PREVIEW_SCHEMA_VERSION.to_string(), methodology: RGB_PREVIEW_METHODOLOGY_VERSION.to_string(),
        source_color_model: "pdfium_display_rgb".to_string(), approximate: true, color_managed: false,
        output_intent_applied: false, spot_colors_preserved: false, overprint_simulated: false,
        rendering_intent: "not_applied".to_string(),
        warnings: vec!["Derived from PDFium display RGB pixels; not sRGB-assumed, color-managed separation, real ink measurement, or RIP output".to_string()],
    }
}

fn render_preview_rgba(
    file_path: &str,
    password: Option<&str>,
    page_index: u32,
    scale: f32,
    cancellation: PrepressCancellation<'_>,
) -> Result<image::RgbaImage, SessionJobOperationError> {
    cancellation.check()?;
    if !scale.is_finite() || !(MIN_PREVIEW_SCALE..=MAX_PREVIEW_SCALE).contains(&scale) {
        return Err(preview_error(
            "RGB_PREVIEW_INVALID_SCALE",
            "Scale must be finite and within the safe preview range",
        ));
    }
    let pdfium = get_pdfium().map_err(SessionJobOperationError::from)?;
    let document =
        open_pdfium_doc(&pdfium, file_path, password).map_err(SessionJobOperationError::from)?;
    if page_index >= u32::from(document.pages().len()) {
        return Err(preview_error(
            "RGB_PREVIEW_INVALID_PAGE",
            "Physical page index is outside the document",
        ));
    }
    let page = document
        .pages()
        .get(u16::try_from(page_index).map_err(|_| {
            preview_error(
                "RGB_PREVIEW_INVALID_PAGE",
                "Physical page index is outside PDFium range",
            )
        })?)
        .map_err(|error| {
            preview_error(
                "RGB_PREVIEW_INVALID_PAGE",
                format!("Page could not be loaded: {error}"),
            )
        })?;
    let target_width = f64::from(page.width().value) * f64::from(scale);
    let target_height = f64::from(page.height().value) * f64::from(scale);
    if !target_width.is_finite()
        || !target_height.is_finite()
        || target_width < 1.0
        || target_height < 1.0
        || target_width > f64::from(MAX_PREVIEW_DIMENSION)
        || target_height > f64::from(MAX_PREVIEW_DIMENSION)
    {
        return Err(preview_error(
            "RGB_PREVIEW_PIXEL_LIMIT_EXCEEDED",
            "Preview dimensions exceed the safe render limit",
        ));
    }
    let estimated_pixels = (target_width.ceil() as u64)
        .checked_mul(target_height.ceil() as u64)
        .ok_or_else(|| {
            preview_error(
                "RGB_PREVIEW_PIXEL_LIMIT_EXCEEDED",
                "Preview pixel count overflow",
            )
        })?;
    if estimated_pixels > MAX_PREVIEW_PIXELS {
        return Err(preview_error(
            "RGB_PREVIEW_PIXEL_LIMIT_EXCEEDED",
            "Preview pixel count exceeds the safe render limit",
        ));
    }
    cancellation.check()?;
    let bitmap = page
        .render_with_config(
            &PdfRenderConfig::new()
                .set_target_width(target_width.round() as i32)
                .set_clear_color(PdfColor::WHITE),
        )
        .map_err(|error| {
            preview_error(
                "RGB_PREVIEW_RENDER_FAILED",
                format!("PDFium display render failed: {error}"),
            )
        })?;
    cancellation.check()?;
    let image = bitmap.as_image().to_rgba8();
    let actual_pixels = u64::from(image.width())
        .checked_mul(u64::from(image.height()))
        .ok_or_else(|| {
            preview_error(
                "RGB_PREVIEW_PIXEL_LIMIT_EXCEEDED",
                "Rendered pixel count overflow",
            )
        })?;
    if image.width() > MAX_PREVIEW_DIMENSION
        || image.height() > MAX_PREVIEW_DIMENSION
        || actual_pixels > MAX_PREVIEW_PIXELS
    {
        return Err(preview_error(
            "RGB_PREVIEW_PIXEL_LIMIT_EXCEEDED",
            "Rendered bitmap exceeds the safe pixel limit",
        ));
    }
    Ok(image)
}

fn encode_png(
    image: &image::RgbaImage,
    cancellation: PrepressCancellation<'_>,
) -> Result<Vec<u8>, SessionJobOperationError> {
    cancellation.check()?;
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .map_err(|error| {
            preview_error(
                "RGB_PREVIEW_PNG_ENCODE_FAILED",
                format!("Preview PNG encode failed: {error}"),
            )
        })?;
    cancellation.check()?;
    if bytes.len() > MAX_PNG_BYTES {
        return Err(preview_error(
            "RGB_PREVIEW_RESULT_LIMIT_EXCEEDED",
            "Preview PNG byte limit exceeded",
        ));
    }
    Ok(bytes)
}

fn rgb_to_estimated_process(pixel: [u8; 4]) -> [f32; 4] {
    let alpha = f32::from(pixel[3]) / 255.0;
    let composite = |channel: u8| (f32::from(channel) / 255.0) * alpha + (1.0 - alpha);
    let r = composite(pixel[0]);
    let g = composite(pixel[1]);
    let b = composite(pixel[2]);
    let k = 1.0 - r.max(g).max(b);
    if k >= 1.0 - f32::EPSILON {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [
            ((1.0 - r - k) / (1.0 - k)).clamp(0.0, 1.0),
            ((1.0 - g - k) / (1.0 - k)).clamp(0.0, 1.0),
            ((1.0 - b - k) / (1.0 - k)).clamp(0.0, 1.0),
            k.clamp(0.0, 1.0),
        ]
    }
}

fn apply_pdfium_page_geometry(
    file_path: &str,
    boxes: &mut [PageBoxesObservation],
    cancellation: PrepressCancellation<'_>,
) -> Result<(), SessionJobOperationError> {
    cancellation.check()?;
    let pdfium = get_pdfium().map_err(SessionJobOperationError::from)?;
    let document =
        open_pdfium_doc(&pdfium, file_path, None).map_err(SessionJobOperationError::from)?;
    for (index, observation) in boxes.iter_mut().enumerate() {
        if index % 32 == 0 {
            cancellation.check()?;
        }
        let page = document
            .pages()
            .get(u16::try_from(index).map_err(|_| {
                preview_error(
                    "PRINT_INSPECTION_PAGE_LIMIT",
                    "Page index exceeds PDFium range",
                )
            })?)
            .map_err(|error| {
                preview_error(
                    "PRINT_INSPECTION_PAGE_GEOMETRY_FAILED",
                    format!("PDFium page geometry unavailable: {error}"),
                )
            })?;
        observation.rotation_degrees = match page.rotation().unwrap_or(PdfPageRenderRotation::None)
        {
            PdfPageRenderRotation::None => 0,
            PdfPageRenderRotation::Degrees90 => 90,
            PdfPageRenderRotation::Degrees180 => 180,
            PdfPageRenderRotation::Degrees270 => 270,
        };
        let boundaries = page.boundaries();
        apply_pdfium_box(&mut observation.media_box, boundaries.media().ok());
        apply_pdfium_box(&mut observation.crop_box, boundaries.crop().ok());
        apply_pdfium_box(&mut observation.bleed_box, boundaries.bleed().ok());
        apply_pdfium_box(&mut observation.trim_box, boundaries.trim().ok());
        apply_pdfium_box(&mut observation.art_box, boundaries.art().ok());
    }
    Ok(())
}

fn apply_pdfium_box(observation: &mut PageBoxObservation, boundary: Option<PdfPageBoundaryBox>) {
    if let Some(boundary) = boundary {
        let rect = boundary.bounds;
        let coordinates = [
            rect.left().value,
            rect.bottom().value,
            rect.right().value,
            rect.top().value,
        ];
        observation.coordinates = Some(coordinates);
        observation.valid_geometry = valid_box(coordinates);
        observation.non_zero_origin = coordinates[0] != 0.0 || coordinates[1] != 0.0;
        observation.negative_origin = coordinates[0] < 0.0 || coordinates[1] < 0.0;
    }
}

fn inspect_icc_stream(
    stream: &Stream,
    object_id: Option<ObjectId>,
    total_before: usize,
) -> IccProfileObservation {
    let mut warnings = Vec::new();
    let encoded_length = stream.content.len() as u64;
    let decoded = if stream.content.len() > MAX_SINGLE_ICC_BYTES {
        warnings.push("Encoded ICC stream exceeds the single-profile limit".to_string());
        None
    } else {
        let filters = stream.filters().unwrap_or_default();
        if filters.is_empty() {
            Some(stream.content.clone())
        } else if filters.len() == 1
            && (filters[0] == b"FlateDecode" || filters[0] == b"Fl")
            && !stream.dict.has(b"DecodeParms")
        {
            match miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(
                &stream.content,
                MAX_SINGLE_ICC_BYTES,
            ) {
                Ok(bytes) => Some(bytes),
                Err(_) => {
                    warnings
                        .push("Bounded Flate ICC decode failed or exceeded the limit".to_string());
                    None
                }
            }
        } else {
            warnings.push(
                "ICC stream filter chain is not supported by the bounded basic-header decoder"
                    .to_string(),
            );
            None
        }
    };
    let over_total = decoded
        .as_ref()
        .is_some_and(|bytes| total_before.saturating_add(bytes.len()) > MAX_TOTAL_ICC_BYTES);
    let header = decoded
        .as_deref()
        .filter(|bytes| bytes.len() >= 128 && !over_total);
    if decoded.as_ref().is_some_and(|bytes| bytes.len() < 128) {
        warnings.push("Decoded ICC stream is shorter than the minimum header".to_string());
    }
    let tag = |offset: usize| header.map(|bytes| ascii_tag(&bytes[offset..offset + 4]));
    IccProfileObservation {
        object_id: object_id.map(object_id_string),
        inspection_level: "basic_header_only".to_string(),
        stream_present: true,
        encoded_length,
        decoded_length: decoded.as_ref().map(|bytes| bytes.len() as u64),
        sha256: header.map(|_| hex_digest(decoded.as_ref().expect("header implies decoded bytes"))),
        component_count: stream
            .dict
            .get(b"N")
            .ok()
            .and_then(|value| value.as_i64().ok()),
        alternate_color_space: stream
            .dict
            .get(b"Alternate")
            .ok()
            .map(|value| color_space_family(value).0),
        declared_size: header
            .map(|bytes| u32::from_be_bytes(bytes[0..4].try_into().expect("four-byte slice"))),
        signature_acsp: header.map(|bytes| &bytes[36..40] == b"acsp"),
        profile_version: header
            .map(|bytes| format!("{}.{}.{}", bytes[8], bytes[9] >> 4, bytes[9] & 0x0f)),
        device_class: tag(12),
        input_color_space: tag(16),
        pcs: tag(20),
        status: if header.is_some() {
            "observed_basic_header"
        } else {
            "incomplete"
        }
        .to_string(),
        warnings,
    }
}

fn color_space_family(object: &Object) -> (String, Option<&[Object]>) {
    match object {
        Object::Name(name) => (normalize_color_space_name(&pdf_name(name)), None),
        Object::Array(values) => {
            let name = values
                .first()
                .and_then(object_name)
                .unwrap_or_else(|| "unknown".to_string());
            let mut family = normalize_color_space_name(&name);
            if family == "device_n"
                && values
                    .get(4)
                    .and_then(|value| value.as_dict().ok())
                    .and_then(|dict| dict.get(b"Subtype").ok())
                    .and_then(object_name)
                    .is_some_and(|value| value == "NChannel")
            {
                family = "n_channel".to_string();
            }
            (family, Some(values))
        }
        Object::Reference(_) => ("indirect_unresolved".to_string(), None),
        _ => ("unknown_custom".to_string(), None),
    }
}

fn normalize_color_space_name(name: &str) -> String {
    match name {
        "DeviceGray" | "G" => "device_gray",
        "DeviceRGB" | "RGB" => "device_rgb",
        "DeviceCMYK" | "CMYK" => "device_cmyk",
        "CalGray" => "cal_gray",
        "CalRGB" => "cal_rgb",
        "Lab" => "lab",
        "ICCBased" => "icc_based",
        "Indexed" | "I" => "indexed",
        "Pattern" => "pattern",
        "Separation" => "separation",
        "DeviceN" => "device_n",
        _ => "unknown_custom",
    }
    .to_string()
}

fn object_name(object: &Object) -> Option<String> {
    object.as_name().ok().map(pdf_name)
}
fn object_text(object: &Object) -> Option<String> {
    object
        .as_str()
        .ok()
        .map(|bytes| String::from_utf8_lossy(bytes).chars().take(1024).collect())
}
fn pdf_name(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).chars().take(256).collect()
}
fn object_id_string(id: ObjectId) -> String {
    format!("{} {} R", id.0, id.1)
}
fn ascii_tag(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte)
            } else {
                '?'
            }
        })
        .collect()
}
fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn is_subset_name(name: &str) -> bool {
    name.as_bytes()
        .get(0..7)
        .is_some_and(|value| value[0..6].iter().all(u8::is_ascii_uppercase) && value[6] == b'+')
}

fn parse_box(object: &Object) -> Option<[f32; 4]> {
    let values = object.as_array().ok()?;
    if values.len() != 4 {
        return None;
    }
    let mut coords = [0.0_f32; 4];
    for (index, value) in values.iter().enumerate() {
        coords[index] = match value {
            Object::Integer(value) => *value as f32,
            Object::Real(value) => *value,
            _ => return None,
        };
    }
    Some(coords)
}

fn box_observation(coordinates: Option<[f32; 4]>, source: String) -> PageBoxObservation {
    let valid_geometry = coordinates.is_some_and(valid_box);
    let non_zero_origin = coordinates.is_some_and(|value| value[0] != 0.0 || value[1] != 0.0);
    let negative_origin = coordinates.is_some_and(|value| value[0] < 0.0 || value[1] < 0.0);
    PageBoxObservation {
        coordinates,
        source,
        valid_geometry,
        non_zero_origin,
        negative_origin,
    }
}
fn valid_box(value: [f32; 4]) -> bool {
    value.iter().all(|coordinate| coordinate.is_finite())
        && value[2] > value[0]
        && value[3] > value[1]
}
fn contains_box(outer: [f32; 4], inner: [f32; 4]) -> bool {
    inner[0] >= outer[0] && inner[1] >= outer[1] && inner[2] <= outer[2] && inner[3] <= outer[3]
}
fn preview_error(code: &'static str, message: impl Into<String>) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code(code, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    fn document_with_resources(resources: Object) -> (Document, ObjectId, ObjectId) {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let page_id = document.new_object_id();
        let catalog_id = document.new_object_id();
        document.objects.insert(
            page_id,
            Object::Dictionary(lopdf::dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 200.into(), 300.into()],
                "Resources" => resources,
            }),
        );
        document.objects.insert(
            pages_id,
            Object::Dictionary(lopdf::dictionary! {
                "Type" => "Pages",
                "Kids" => vec![Object::Reference(page_id)],
                "Count" => 1,
            }),
        );
        document.objects.insert(
            catalog_id,
            Object::Dictionary(lopdf::dictionary! { "Type" => "Catalog", "Pages" => pages_id }),
        );
        document.trailer.set("Root", catalog_id);
        (document, page_id, pages_id)
    }

    #[test]
    fn deterministic_rgb_formula_covers_primary_pixels() {
        assert_eq!(
            rgb_to_estimated_process([255, 255, 255, 255]),
            [0.0, 0.0, 0.0, 0.0]
        );
        assert_eq!(
            rgb_to_estimated_process([0, 0, 0, 255]),
            [0.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(
            rgb_to_estimated_process([255, 0, 0, 255]),
            [0.0, 1.0, 1.0, 0.0]
        );
        assert_eq!(
            rgb_to_estimated_process([0, 255, 0, 255]),
            [1.0, 0.0, 1.0, 0.0]
        );
        assert_eq!(
            rgb_to_estimated_process([0, 0, 255, 255]),
            [1.0, 1.0, 0.0, 0.0]
        );
        assert_eq!(
            rgb_to_estimated_process([0, 255, 255, 255]),
            [1.0, 0.0, 0.0, 0.0]
        );
        assert_eq!(
            rgb_to_estimated_process([255, 0, 255, 255]),
            [0.0, 1.0, 0.0, 0.0]
        );
        assert_eq!(
            rgb_to_estimated_process([255, 255, 0, 255]),
            [0.0, 0.0, 1.0, 0.0]
        );
        let gradient = rgb_to_estimated_process([128, 64, 32, 255]);
        assert!(gradient.iter().all(|value| (0.0..=1.0).contains(value)));
    }

    #[test]
    fn color_space_inventory_recognizes_required_families() {
        for (name, expected) in [
            ("DeviceGray", "device_gray"),
            ("DeviceRGB", "device_rgb"),
            ("DeviceCMYK", "device_cmyk"),
            ("CalGray", "cal_gray"),
            ("CalRGB", "cal_rgb"),
            ("Lab", "lab"),
            ("ICCBased", "icc_based"),
            ("Indexed", "indexed"),
            ("Pattern", "pattern"),
            ("Separation", "separation"),
            ("DeviceN", "device_n"),
        ] {
            assert_eq!(normalize_color_space_name(name), expected);
        }
        let n_channel = Object::Array(vec![
            Object::Name(b"DeviceN".to_vec()),
            Object::Array(vec![Object::Name(b"Orange".to_vec())]),
            Object::Name(b"DeviceCMYK".to_vec()),
            Object::Null,
            Object::Dictionary(lopdf::dictionary! { "Subtype" => "NChannel" }),
        ]);
        assert_eq!(color_space_family(&n_channel).0, "n_channel");
        assert_eq!(normalize_color_space_name("VendorSpace"), "unknown_custom");
    }

    #[test]
    fn malformed_and_negative_boxes_are_observations_not_conformance_claims() {
        assert!(valid_box([-10.0, -20.0, 100.0, 200.0]));
        assert!(!valid_box([0.0, 0.0, 0.0, 100.0]));
        assert!(!contains_box([0.0, 0.0, 10.0, 10.0], [-1.0, 0.0, 5.0, 5.0]));
    }

    #[test]
    fn bounded_icc_header_is_basic_only() {
        let mut bytes = vec![0_u8; 128];
        bytes[0..4].copy_from_slice(&128_u32.to_be_bytes());
        bytes[8] = 4;
        bytes[12..16].copy_from_slice(b"prtr");
        bytes[16..20].copy_from_slice(b"CMYK");
        bytes[20..24].copy_from_slice(b"Lab ");
        bytes[36..40].copy_from_slice(b"acsp");
        let stream = Stream::new(lopdf::dictionary! { "N" => 4 }, bytes);
        let observation = inspect_icc_stream(&stream, Some((9, 0)), 0);
        assert_eq!(observation.inspection_level, "basic_header_only");
        assert_eq!(observation.signature_acsp, Some(true));
        assert_eq!(observation.input_color_space.as_deref(), Some("CMYK"));
        assert_eq!(observation.status, "observed_basic_header");
    }

    #[test]
    fn deterministic_resource_corpus_covers_color_spaces_spots_and_output_intent() {
        let mut icc_bytes = vec![0_u8; 128];
        icc_bytes[0..4].copy_from_slice(&128_u32.to_be_bytes());
        icc_bytes[36..40].copy_from_slice(b"acsp");
        let icc = Stream::new(
            lopdf::dictionary! { "N" => 4, "Alternate" => "DeviceCMYK" },
            icc_bytes,
        );
        let color_spaces = lopdf::dictionary! {
            "Gray" => "DeviceGray", "Rgb" => "DeviceRGB", "Cmyk" => "DeviceCMYK",
            "CalG" => vec![Object::Name(b"CalGray".to_vec()), Object::Dictionary(Dictionary::new())],
            "CalR" => vec![Object::Name(b"CalRGB".to_vec()), Object::Dictionary(Dictionary::new())],
            "Lab" => vec![Object::Name(b"Lab".to_vec()), Object::Dictionary(Dictionary::new())],
            "Icc" => vec![Object::Name(b"ICCBased".to_vec()), Object::Stream(icc)],
            "Idx" => vec![Object::Name(b"Indexed".to_vec()), Object::Name(b"DeviceRGB".to_vec()), 1.into(), Object::String(vec![0, 0, 0, 255, 255, 255], lopdf::StringFormat::Literal)],
            "Pat" => vec![Object::Name(b"Pattern".to_vec()), Object::Name(b"DeviceRGB".to_vec())],
            "Spot" => vec![Object::Name(b"Separation".to_vec()), Object::Name(b"Varnish".to_vec()), Object::Name(b"DeviceCMYK".to_vec()), Object::Null],
            "DeviceN" => vec![Object::Name(b"DeviceN".to_vec()), Object::Array(vec![Object::Name(b"All".to_vec()), Object::Name(b"None".to_vec())]), Object::Name(b"DeviceCMYK".to_vec()), Object::Null],
        };
        let resources = Object::Dictionary(lopdf::dictionary! { "ColorSpace" => color_spaces });
        let (mut document, _, _) = document_with_resources(resources);
        let output_profile = Stream::new(lopdf::dictionary! { "N" => 4 }, vec![0_u8; 128]);
        document.catalog_mut().unwrap().set("OutputIntents", vec![Object::Dictionary(lopdf::dictionary! {
            "Type" => "OutputIntent", "S" => "GTS_PDFX", "OutputConditionIdentifier" => Object::string_literal("fixture"),
            "DestOutputProfile" => Object::Stream(output_profile),
        })]);
        let scanner = InspectionScanner::new(&document, PrepressCancellation::none())
            .scan()
            .unwrap();
        let families = scanner
            .color_spaces
            .values()
            .map(|value| value.family.as_str())
            .collect::<BTreeSet<_>>();
        for family in [
            "device_gray",
            "device_rgb",
            "device_cmyk",
            "cal_gray",
            "cal_rgb",
            "lab",
            "icc_based",
            "indexed",
            "pattern",
            "separation",
            "device_n",
        ] {
            assert!(families.contains(family), "missing {family}");
        }
        assert_eq!(scanner.output_intents.len(), 1);
        assert_eq!(scanner.icc_profiles.len(), 2);
        assert_eq!(scanner.spot_colors.len(), 2);
        assert!(scanner
            .spot_colors
            .iter()
            .any(|spot| spot.contains_all && spot.contains_none));
    }

    #[test]
    fn nested_form_resource_cycle_is_bounded_and_inherited_boxes_are_observed() {
        let (mut document, page_id, pages_id) = document_with_resources(Object::Null);
        let resources_id = document.new_object_id();
        let form_id = document.new_object_id();
        document.objects.insert(resources_id, Object::Dictionary(lopdf::dictionary! {
            "XObject" => lopdf::dictionary! { "Loop" => form_id },
            "Pattern" => lopdf::dictionary! { "P" => Object::Dictionary(lopdf::dictionary! { "Resources" => resources_id }) },
            "Shading" => lopdf::dictionary! { "S" => Object::Dictionary(lopdf::dictionary! { "ColorSpace" => "DeviceRGB" }) },
        }));
        document.objects.insert(
            form_id,
            Object::Stream(Stream::new(
                lopdf::dictionary! {
                    "Subtype" => "Form", "Resources" => resources_id,
                },
                Vec::new(),
            )),
        );
        document
            .get_object_mut(page_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Resources", resources_id);
        document
            .get_object_mut(page_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .remove(b"MediaBox");
        document
            .get_object_mut(pages_id)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set(
                "MediaBox",
                vec![(-10).into(), (-20).into(), 200.into(), 300.into()],
            );
        let scanner = InspectionScanner::new(&document, PrepressCancellation::none())
            .scan()
            .unwrap();
        assert!(scanner
            .warnings
            .iter()
            .any(|warning| warning.contains("cycle")));
        assert_eq!(scanner.page_boxes[0].media_box.source, "inherited");
        assert!(scanner.page_boxes[0].media_box.negative_origin);
        assert!(scanner
            .color_spaces
            .values()
            .any(|value| value.family == "device_rgb"));
    }

    #[test]
    fn structural_corpus_covers_images_patterns_shadings_fonts_transparency_spots_and_boxes() {
        let image = Stream::new(
            lopdf::dictionary! {
                "Subtype" => "Image", "ColorSpace" => "DeviceCMYK", "SMask" => Object::Null,
            },
            Vec::new(),
        );
        let font = Object::Dictionary(lopdf::dictionary! {
            "Subtype" => "Type1",
            "BaseFont" => "ABCDEF+FixtureFont",
            "FontDescriptor" => Object::Dictionary(lopdf::dictionary! {
                "FontFile2" => Object::Stream(Stream::new(Dictionary::new(), vec![1, 2, 3])),
            }),
        });
        let spot = vec![
            Object::Name(b"Separation".to_vec()),
            Object::Name(b"Varnish".to_vec()),
            Object::Null,
            Object::Null,
        ];
        let resources = Object::Dictionary(lopdf::dictionary! {
            "ColorSpace" => lopdf::dictionary! { "SpotA" => spot.clone(), "SpotB" => spot },
            "XObject" => lopdf::dictionary! { "Image" => Object::Stream(image) },
            "Pattern" => lopdf::dictionary! { "PatternA" => Object::Dictionary(lopdf::dictionary! {
                "Resources" => Object::Dictionary(lopdf::dictionary! { "ColorSpace" => lopdf::dictionary! { "PatternLab" => vec![Object::Name(b"Lab".to_vec()), Object::Dictionary(Dictionary::new())] } }),
            }) },
            "Shading" => lopdf::dictionary! { "Shade" => Object::Dictionary(lopdf::dictionary! { "ColorSpace" => "DeviceRGB" }) },
            "ExtGState" => lopdf::dictionary! { "GS" => Object::Dictionary(lopdf::dictionary! { "CA" => 0.5, "ca" => 0.5, "BM" => "Multiply", "OP" => true, "op" => true, "OPM" => 1 }) },
            "Font" => lopdf::dictionary! { "F1" => font },
        });
        let (mut document, page_id, _) = document_with_resources(resources);
        let page = document
            .get_object_mut(page_id)
            .unwrap()
            .as_dict_mut()
            .unwrap();
        page.set(
            "CropBox",
            vec![10.into(), 10.into(), 190.into(), 290.into()],
        );
        page.set("BleedBox", vec![5.into(), 5.into(), 195.into(), 295.into()]);
        page.set(
            "TrimBox",
            vec![15.into(), 15.into(), 185.into(), 285.into()],
        );
        page.set("ArtBox", vec![20.into(), 20.into(), 180.into(), 280.into()]);
        page.set("Rotate", 90);

        let scanner = InspectionScanner::new(&document, PrepressCancellation::none())
            .scan()
            .unwrap();
        assert_eq!(scanner.xobjects_scanned, 1);
        assert_eq!(scanner.patterns_scanned, 1);
        assert_eq!(scanner.shadings_scanned, 1);
        assert!(scanner
            .color_spaces
            .values()
            .any(|item| item.family == "device_cmyk"));
        assert!(scanner
            .color_spaces
            .values()
            .any(|item| item.family == "lab"));
        assert!(scanner
            .transparency
            .iter()
            .any(|item| item.kind == "image_mask"));
        assert!(scanner
            .transparency
            .iter()
            .any(|item| item.kind == "ext_g_state" && item.keys.contains(&"OPM".to_string())));
        assert!(scanner.fonts[0].embedded_stream_present);
        assert!(scanner.fonts[0].subset_name_indication);
        assert!(scanner.spot_colors.iter().any(|item| item
            .warnings
            .iter()
            .any(|warning| warning.contains("Duplicate/conflicting"))));
        assert_eq!(scanner.page_boxes[0].rotation_degrees, 90);
        assert!(scanner.page_boxes[0].trim_box.valid_geometry);
    }

    #[test]
    fn output_intent_corpus_reports_malformed_external_and_multiple_without_validation_claims() {
        let (mut document, _, _) = document_with_resources(Object::Dictionary(Dictionary::new()));
        document.catalog_mut().unwrap().set("OutputIntents", vec![
            Object::Dictionary(lopdf::dictionary! {
                "Type" => "NotOutputIntent", "S" => "GTS_PDFX", "DestOutputProfile" => Object::Stream(Stream::new(lopdf::dictionary! { "N" => 4 }, vec![0; 12])),
            }),
            Object::Dictionary(lopdf::dictionary! {
                "Type" => "OutputIntent", "DestOutputProfileRef" => Object::Dictionary(Dictionary::new()),
            }),
        ]);
        let scanner = InspectionScanner::new(&document, PrepressCancellation::none())
            .scan()
            .unwrap();
        assert_eq!(scanner.output_intents.len(), 2);
        assert!(scanner.output_intents[0]
            .warnings
            .iter()
            .any(|warning| warning.contains("Type is not")));
        assert!(scanner.output_intents[1].has_external_profile_reference);
        assert!(!scanner.output_intents[1].has_dest_output_profile);
        assert!(scanner
            .warnings
            .iter()
            .any(|warning| warning.contains("Multiple OutputIntent")));
        assert!(scanner
            .icc_profiles
            .iter()
            .any(|profile| profile.status != "observed_basic_header"));
    }

    #[test]
    fn traversal_and_resource_limits_fail_closed_as_incomplete() {
        let (mut document, _, _) = document_with_resources(Object::Dictionary(Dictionary::new()));
        let mut next = Object::Name(b"DeviceRGB".to_vec());
        for _ in 0..MAX_TRAVERSAL_DEPTH + 8 {
            let id = document.add_object(next);
            next = Object::Reference(id);
        }
        let mut scanner = InspectionScanner::new(&document, PrepressCancellation::none());
        assert!(scanner
            .resolve_object(&next)
            .unwrap_err()
            .contains("depth limit"));
        scanner.resource_references = MAX_RESOURCE_REFERENCES;
        assert!(!scanner.touch_reference(None));
        assert!(scanner.truncated);
    }

    #[test]
    fn preview_inputs_and_cancellation_fail_with_typed_errors_before_output() {
        let identity = PrepressResultIdentity {
            session_id: "s".into(),
            content_generation: 1,
            physical_page_index: 0,
            request_id: "r".into(),
            methodology_version: RGB_PREVIEW_METHODOLOGY_VERSION.into(),
        };
        let invalid_threshold = generate_rgb_derived_coverage_preview(
            "missing.pdf".into(),
            None,
            identity.clone(),
            401.0,
            1.0,
            PrepressCancellation::none(),
        );
        assert!(
            matches!(invalid_threshold, Err(SessionJobOperationError::Failed(ref failure)) if failure.code == "RGB_PREVIEW_INVALID_THRESHOLD")
        );
        let invalid_scale = generate_rgb_derived_process_preview(
            "missing.pdf".into(),
            None,
            identity,
            f32::NAN,
            PrepressCancellation::none(),
        );
        assert!(
            matches!(invalid_scale, Err(SessionJobOperationError::Failed(ref failure)) if failure.code == "RGB_PREVIEW_INVALID_SCALE")
        );

        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("cancel.requested");
        std::fs::write(&marker, []).unwrap();
        let cancelled = PrepressCancellation::marker(&marker).check();
        assert!(
            matches!(cancelled, Err(SessionJobOperationError::Failed(ref failure)) if failure.code == "SESSION_JOB_WORKER_CANCELLED")
        );
    }
}
