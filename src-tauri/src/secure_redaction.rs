use crate::document_session::SessionJobOperationError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

pub const REDACTION_SCHEMA_VERSION: &str = "1.0.0";
/// Version of the independent, worker-only secure-redaction verification protocol.
/// This is deliberately separate from the wire schema version so a future verifier
/// can tighten its observations without accepting a partial v1 check list.
pub const SECURE_REDACTION_VERIFICATION_VERSION: &str = "secure_redaction_verifier_v1";
pub const REQUIRED_SECURE_REDACTION_VERIFICATION_CHECKS: [&str; 14] = [
    "SOURCE_PLAN_BINDING",
    "TRANSFORM_REPORT_CONSISTENCY",
    "ARTIFACT_PATH_INTEGRITY",
    "ARTIFACT_DIGEST_AND_SIZE",
    "DUAL_PARSER_REOPEN",
    "FULL_REWRITE_STRUCTURE",
    "FORBIDDEN_DOCUMENT_STRUCTURES_ABSENT",
    "PAGE_COUNT_AND_STRATEGY",
    "PAGE_GEOMETRY",
    "AFFECTED_PAGE_IMAGE_ONLY",
    "AFFECTED_PAGE_INTERACTIVE_CONTENT_ABSENT",
    "AFFECTED_PAGE_REDACTION_PIXEL_COVERAGE",
    "UNAFFECTED_PAGE_EQUIVALENCE",
    "ARTIFACT_FINAL_DIGEST_STABILITY",
];
pub const MAX_REGIONS_PER_INPUT: usize = 1000;
pub const MAX_REGION_ID_LEN: usize = 128;
pub const MAX_SOURCE_LEN: usize = 256;
pub const MAX_SOURCE_REGION_IDS: usize = 100;
pub const MAX_CANONICAL_REGIONS_PER_PAGE: usize = 1000;
pub const MAX_PAGE_PLANS: usize = 1000;
pub const MAX_BLOCKERS: usize = 100;
pub const MAX_WARNINGS: usize = 500;
pub const MAX_VERIFICATION_CHECKS: usize = 100;

pub const MAX_BLOCKER_CODE_LEN: usize = 128;
pub const MAX_WARNING_CODE_LEN: usize = 128;
pub const MAX_USER_SAFE_MESSAGE_LEN: usize = 2048;
pub const MAX_DETAILS_LEN: usize = 4096;
pub const MAX_IDENTITY_LEN: usize = 128;
pub const MAX_VERIFICATION_CHECK_CODE_LEN: usize = 256;
pub const MAX_VERIFICATION_CHECK_MESSAGE_LEN: usize = 2048;
pub const MAX_VERIFICATION_CHECK_DETAILS_LEN: usize = 4096;
pub const MAX_PATH_LEN: usize = 4096;

pub const MIN_RASTER_DPI: u32 = 72;
pub const MAX_RASTER_DPI: u32 = 600;
pub const MAX_RASTER_DIMENSION: u32 = 16384;
pub const MIN_SAFETY_MARGIN_POINTS: f64 = 0.0;
pub const MAX_SAFETY_MARGIN_POINTS: f64 = 36.0;

pub const MAX_PIXELS_PER_PAGE: u64 = 100_000_000;
pub const MAX_TOTAL_ESTIMATED_PIXELS: u64 = 1_000_000_000;
pub const MAX_REPORT_SIZE_BYTES: usize = 10_485_760;
/// Maximum size of the internal, non-published secure-redaction artifact.
///
/// This is deliberately below the worker's address-space ceiling and is a
/// separate policy from the `u64` wire representation of a file length. The
/// latter remains `u64` because filesystem metadata is `u64`; this bound is
/// what makes the value safely and exactly representable by the JS contract.
pub const MAX_SECURE_REDACTION_OUTPUT_BYTES: u64 = 1073741824;

// ---------------------------------------------------------------------------
// 0. Centralized Typed Error Family
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SecureRedactionErrorCode {
    RedactionInvalidPolicy,
    RedactionPolicyVersionUnsupported,
    RedactionInvalidRegion,
    RedactionRegionOutOfBounds,
    RedactionDuplicateRegionId,
    RedactionRegionLimitExceeded,
    RedactionPageNotFound,
    RedactionPageGeometryMismatch,
    RedactionPlanStale,
    RedactionPlanDigestMismatch,
    RedactionBlockedDocumentFeature,
    RedactionEncryptedInputUnsupported,
    RedactionSignedInputUnsupported,
    RedactionXfaInputUnsupported,
    RedactionResourceLimitExceeded,
}

impl SecureRedactionErrorCode {
    /// Exhaustive contract enumeration. Keep this list in the same order as the
    /// versioned contract manifest so tests can prove exact wire parity.
    pub const ALL: [Self; 15] = [
        Self::RedactionInvalidPolicy,
        Self::RedactionPolicyVersionUnsupported,
        Self::RedactionInvalidRegion,
        Self::RedactionRegionOutOfBounds,
        Self::RedactionDuplicateRegionId,
        Self::RedactionRegionLimitExceeded,
        Self::RedactionPageNotFound,
        Self::RedactionPageGeometryMismatch,
        Self::RedactionPlanStale,
        Self::RedactionPlanDigestMismatch,
        Self::RedactionBlockedDocumentFeature,
        Self::RedactionEncryptedInputUnsupported,
        Self::RedactionSignedInputUnsupported,
        Self::RedactionXfaInputUnsupported,
        Self::RedactionResourceLimitExceeded,
    ];

    pub fn code_str(self) -> &'static str {
        match self {
            Self::RedactionInvalidPolicy => "REDACTION_INVALID_POLICY",
            Self::RedactionPolicyVersionUnsupported => "REDACTION_POLICY_VERSION_UNSUPPORTED",
            Self::RedactionInvalidRegion => "REDACTION_INVALID_REGION",
            Self::RedactionRegionOutOfBounds => "REDACTION_REGION_OUT_OF_BOUNDS",
            Self::RedactionDuplicateRegionId => "REDACTION_DUPLICATE_REGION_ID",
            Self::RedactionRegionLimitExceeded => "REDACTION_REGION_LIMIT_EXCEEDED",
            Self::RedactionPageNotFound => "REDACTION_PAGE_NOT_FOUND",
            Self::RedactionPageGeometryMismatch => "REDACTION_PAGE_GEOMETRY_MISMATCH",
            Self::RedactionPlanStale => "REDACTION_PLAN_STALE",
            Self::RedactionPlanDigestMismatch => "REDACTION_PLAN_DIGEST_MISMATCH",
            Self::RedactionBlockedDocumentFeature => "REDACTION_BLOCKED_DOCUMENT_FEATURE",
            Self::RedactionEncryptedInputUnsupported => "REDACTION_ENCRYPTED_INPUT_UNSUPPORTED",
            Self::RedactionSignedInputUnsupported => "REDACTION_SIGNED_INPUT_UNSUPPORTED",
            Self::RedactionXfaInputUnsupported => "REDACTION_XFA_INPUT_UNSUPPORTED",
            Self::RedactionResourceLimitExceeded => "REDACTION_RESOURCE_LIMIT_EXCEEDED",
        }
    }
}

pub fn make_redaction_error(
    code: SecureRedactionErrorCode,
    message: impl Into<String>,
) -> SessionJobOperationError {
    SessionJobOperationError::failed_with_code(code.code_str(), message)
}

pub fn validate_sha256_digest(
    digest: &str,
    field_name: &str,
) -> Result<(), SessionJobOperationError> {
    let trimmed = digest.trim();
    if trimmed.len() != 64 || digest.as_bytes().len() != 64 {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPlanDigestMismatch,
            format!(
                "{} must be exactly 64 lower-case hex characters",
                field_name
            ),
        ));
    }
    for b in trimmed.bytes() {
        if !b.is_ascii_hexdigit() || (b.is_ascii_alphabetic() && !b.is_ascii_lowercase()) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPlanDigestMismatch,
                format!(
                    "{} contains invalid non-lowercase-hex characters",
                    field_name
                ),
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 1. Region Input Contract
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NormalizedTopLeftRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RedactionRegionInput {
    pub id: String,
    pub page_index: u32,
    pub normalized_top_left_rect: NormalizedTopLeftRect,
    pub source: String,
}

pub fn validate_redaction_region_input(
    input: &RedactionRegionInput,
) -> Result<(), SessionJobOperationError> {
    let trimmed_id = input.id.trim();
    if trimmed_id.is_empty() || input.id.as_bytes().len() > MAX_REGION_ID_LEN {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidRegion,
            "Region ID must be non-empty, non-whitespace, and at most 128 UTF-8 bytes",
        ));
    }
    let trimmed_source = input.source.trim();
    if trimmed_source.is_empty() || input.source.as_bytes().len() > MAX_SOURCE_LEN {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidRegion,
            "Region source must be non-empty, non-whitespace, and at most 256 UTF-8 bytes",
        ));
    }

    let rect = &input.normalized_top_left_rect;
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidRegion,
            "Normalized rectangle coordinates must be finite numbers",
        ));
    }
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidRegion,
            "Normalized rectangle width and height must be strictly positive",
        ));
    }

    let max_x = rect.x + rect.width;
    let max_y = rect.y + rect.height;
    if !max_x.is_finite() || !max_y.is_finite() {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidRegion,
            "Normalized rectangle addition overflow or non-finite result",
        ));
    }

    // Exact bounds check in top-left normalized page space [0.0, 1.0] without epsilon tolerance
    if rect.x < 0.0 || rect.y < 0.0 || max_x > 1.0 || max_y > 1.0 {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionRegionOutOfBounds,
            "Normalized rectangle is out of page bounds [0.0, 1.0]",
        ));
    }

    Ok(())
}

pub fn validate_redaction_region_inputs(
    inputs: &[RedactionRegionInput],
) -> Result<(), SessionJobOperationError> {
    if inputs.len() > MAX_REGIONS_PER_INPUT {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionRegionLimitExceeded,
            format!(
                "Region input count ({}) exceeds maximum limit of {}",
                inputs.len(),
                MAX_REGIONS_PER_INPUT
            ),
        ));
    }

    let mut seen_ids = HashSet::new();
    for input in inputs {
        validate_redaction_region_input(input)?;
        if !seen_ids.insert(&input.id) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionDuplicateRegionId,
                format!("Duplicate region ID found: '{}'", input.id),
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 2. Canonical Internal Models
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PdfPointRect {
    pub left: f64,
    pub bottom: f64,
    pub width: f64,
    pub height: f64,
}

pub fn validate_pdf_point_rect(rect: &PdfPointRect) -> Result<(), SessionJobOperationError> {
    if !rect.left.is_finite()
        || !rect.bottom.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "PdfPointRect coordinates must be finite numbers",
        ));
    }
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "PdfPointRect width and height must be strictly positive",
        ));
    }
    if !(rect.left + rect.width).is_finite() || !(rect.bottom + rect.height).is_finite() {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "PdfPointRect addition overflow or non-finite result",
        ));
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DevicePixelRect {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}

pub fn validate_device_pixel_rect(rect: &DevicePixelRect) -> Result<(), SessionJobOperationError> {
    if rect.width == 0 || rect.height == 0 {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "DevicePixelRect width and height must be strictly positive (zero-area rejected)",
        ));
    }
    if rect.left.checked_add_unsigned(rect.width).is_none()
        || rect.top.checked_add_unsigned(rect.height).is_none()
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "DevicePixelRect addition overflow",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum PageRotation {
    Deg0 = 0,
    Deg90 = 90,
    Deg180 = 180,
    Deg270 = 270,
}

impl PageRotation {
    pub fn as_u16(self) -> u16 {
        self as u16
    }
}

impl TryFrom<u16> for PageRotation {
    type Error = SessionJobOperationError;
    fn try_from(val: u16) -> Result<Self, Self::Error> {
        match val {
            0 => Ok(Self::Deg0),
            90 => Ok(Self::Deg90),
            180 => Ok(Self::Deg180),
            270 => Ok(Self::Deg270),
            other => Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!("Invalid rotation angle: {other}. Expected 0, 90, 180, or 270"),
            )),
        }
    }
}

impl Serialize for PageRotation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u16(self.as_u16())
    }
}

impl<'de> Deserialize<'de> for PageRotation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = u16::deserialize(deserializer)?;
        PageRotation::try_from(raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InspectionFactState {
    Present,
    Absent,
    Unknown,
}

impl InspectionFactState {
    pub fn is_present(self) -> bool {
        matches!(self, Self::Present)
    }

    pub fn is_absent(self) -> bool {
        matches!(self, Self::Absent)
    }

    pub fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown)
    }
}

impl Serialize for InspectionFactState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = match self {
            Self::Present => "present",
            Self::Absent => "absent",
            Self::Unknown => "unknown",
        };
        serializer.serialize_str(s)
    }
}

pub const ADJACENCY_TOLERANCE_POINTS: f64 = 0.0;
pub const MERGE_EDGE_TOUCHING: bool = true;
pub const MERGE_CORNER_TOUCHING: bool = false;
pub const GEOMETRY_EPSILON: f64 = 1e-6;
pub const GEOMETRY_COMPARISON_EPSILON: f64 = 1e-6;

impl<'de> Deserialize<'de> for InspectionFactState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct FactStateVisitor;
        impl<'de> serde::de::Visitor<'de> for FactStateVisitor {
            type Value = InspectionFactState;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a string: 'present', 'absent', or 'unknown'")
            }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match v {
                    "present" => Ok(InspectionFactState::Present),
                    "absent" => Ok(InspectionFactState::Absent),
                    "unknown" => Ok(InspectionFactState::Unknown),
                    _ => Err(E::custom(format!("Invalid InspectionFactState: '{}'", v))),
                }
            }
        }
        deserializer.deserialize_str(FactStateVisitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GeometryProfile {
    StandardEffectiveBoxV1,
}

impl GeometryProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StandardEffectiveBoxV1 => "standard_effective_box_v1",
        }
    }
}

impl Serialize for GeometryProfile {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for GeometryProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "standard_effective_box_v1" => Ok(GeometryProfile::StandardEffectiveBoxV1),
            _ => Err(serde::de::Error::custom(format!(
                "Invalid geometry_profile: '{}'. Expected 'standard_effective_box_v1'",
                s
            ))),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalPageGeometry {
    pub page_index: u32,
    pub media_box: PdfPointRect,
    pub crop_box: Option<PdfPointRect>,
    pub trim_box: Option<PdfPointRect>,
    pub bleed_box: Option<PdfPointRect>,
    pub art_box: Option<PdfPointRect>,
    pub effective_box: PdfPointRect,
    pub rotation: PageRotation,
}

pub fn validate_canonical_page_geometry(
    geom: &CanonicalPageGeometry,
) -> Result<(), SessionJobOperationError> {
    validate_pdf_point_rect(&geom.media_box)?;
    if let Some(crop) = &geom.crop_box {
        validate_pdf_point_rect(crop)?;
        let eps = GEOMETRY_EPSILON;
        if crop.left < geom.media_box.left - eps
            || crop.bottom < geom.media_box.bottom - eps
            || crop.left + crop.width > geom.media_box.left + geom.media_box.width + eps
            || crop.bottom + crop.height > geom.media_box.bottom + geom.media_box.height + eps
        {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                "CropBox extends outside MediaBox boundaries (unsupported geometry)",
            ));
        }
    }
    for (label, boundary) in [
        ("TrimBox", geom.trim_box.as_ref()),
        ("BleedBox", geom.bleed_box.as_ref()),
        ("ArtBox", geom.art_box.as_ref()),
    ] {
        if let Some(boundary) = boundary {
            validate_pdf_point_rect(boundary).map_err(|_| {
                make_redaction_error(
                    SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                    format!("{label} is not a valid finite PDF rectangle"),
                )
            })?;
        }
    }
    validate_pdf_point_rect(&geom.effective_box)?;

    let expected_box = geom.crop_box.as_ref().unwrap_or(&geom.media_box);
    let eps = GEOMETRY_EPSILON;
    if (geom.effective_box.left - expected_box.left).abs() > eps
        || (geom.effective_box.bottom - expected_box.bottom).abs() > eps
        || (geom.effective_box.width - expected_box.width).abs() > eps
        || (geom.effective_box.height - expected_box.height).abs() > eps
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageGeometryMismatch,
            "effective_box must match CropBox (if present) or MediaBox (if CropBox absent)",
        ));
    }
    Ok(())
}

/// The one capability gate used by both planning and the transform worker.
/// PDFium's page-boundary setters accept f32 coordinates, so accepted geometry
/// must survive that conversion within the canonical comparison epsilon.
pub fn evaluate_secure_redaction_rebuild_geometry(
    geom: &CanonicalPageGeometry,
) -> Result<(), SessionJobOperationError> {
    validate_canonical_page_geometry(geom)?;
    for (label, rectangle) in [
        ("MediaBox", Some(&geom.media_box)),
        ("CropBox", geom.crop_box.as_ref()),
        ("TrimBox", geom.trim_box.as_ref()),
        ("BleedBox", geom.bleed_box.as_ref()),
        ("ArtBox", geom.art_box.as_ref()),
    ] {
        let Some(rectangle) = rectangle else { continue };
        for (component, value) in [
            ("left", rectangle.left),
            ("bottom", rectangle.bottom),
            ("right", rectangle.left + rectangle.width),
            ("top", rectangle.bottom + rectangle.height),
        ] {
            let converted = value as f32;
            if !converted.is_finite() || ((converted as f64) - value).abs() > GEOMETRY_EPSILON {
                return Err(make_redaction_error(
                    SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                    format!("{label} {component} cannot be reproduced by the locked PDFium f32 boundary API"),
                ));
            }
        }
    }
    Ok(())
}

fn secure_redaction_rebuild_geometry_blocker(
    geom: &CanonicalPageGeometry,
) -> Option<RedactionBlocker> {
    evaluate_secure_redaction_rebuild_geometry(geom).err().map(|error| RedactionBlocker {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        code: "REDACTION_PAGE_GEOMETRY_MISMATCH".to_string(),
        message: "Affected page geometry cannot be reproduced and verified by the secure PDFium rebuild path".to_string(),
        details: Some(error.to_string()),
    })
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppliedMargins {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalRedactionRegion {
    pub region_id: String,
    pub source_region_ids: Vec<String>,
    pub page_index: u32,
    pub original_normalized_rect: NormalizedTopLeftRect,
    pub base_pdf_point_rect: PdfPointRect,
    pub requested_safety_margin_points: f64,
    #[serde(alias = "canonical_pdf_point_rect")]
    pub applied_pdf_point_rect: PdfPointRect,
    pub applied_margins: AppliedMargins,
    pub boundary_adjusted: bool,
    pub device_pixel_rect: Option<DevicePixelRect>,
}

pub fn compare_canonical_redaction_regions(
    a: &CanonicalRedactionRegion,
    b: &CanonicalRedactionRegion,
) -> std::cmp::Ordering {
    a.page_index
        .cmp(&b.page_index)
        .then_with(|| {
            a.applied_pdf_point_rect
                .bottom
                .partial_cmp(&b.applied_pdf_point_rect.bottom)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| {
            a.applied_pdf_point_rect
                .left
                .partial_cmp(&b.applied_pdf_point_rect.left)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| a.region_id.cmp(&b.region_id))
}

pub fn validate_canonical_redaction_region(
    region: &CanonicalRedactionRegion,
) -> Result<(), SessionJobOperationError> {
    if region.region_id.trim().is_empty() || region.region_id.as_bytes().len() > MAX_REGION_ID_LEN {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidRegion,
            "Canonical region_id must be non-empty and at most 128 UTF-8 bytes",
        ));
    }
    if region.source_region_ids.is_empty() {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidRegion,
            "Canonical source_region_ids list cannot be empty",
        ));
    }
    if region.source_region_ids.len() > MAX_SOURCE_REGION_IDS {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "source_region_ids length ({}) exceeds limit of {}",
                region.source_region_ids.len(),
                MAX_SOURCE_REGION_IDS
            ),
        ));
    }
    let mut seen_sources = HashSet::new();
    let mut prev_source_id: Option<&String> = None;
    for id in &region.source_region_ids {
        if id.trim().is_empty() || id.as_bytes().len() > MAX_REGION_ID_LEN {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidRegion,
                "source_region_id must be non-empty and at most 128 UTF-8 bytes",
            ));
        }
        if !seen_sources.insert(id) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionDuplicateRegionId,
                format!("Duplicate source_region_id '{}' in canonical region", id),
            ));
        }
        if let Some(prev) = prev_source_id {
            if id < prev {
                return Err(make_redaction_error(
                    SecureRedactionErrorCode::RedactionInvalidRegion,
                    "Canonical source_region_ids must be in sorted canonical order",
                ));
            }
        }
        prev_source_id = Some(id);
    }
    validate_redaction_region_input(&RedactionRegionInput {
        id: region.region_id.clone(),
        page_index: region.page_index,
        normalized_top_left_rect: region.original_normalized_rect.clone(),
        source: "canonical".to_string(),
    })?;
    validate_pdf_point_rect(&region.base_pdf_point_rect)?;
    validate_pdf_point_rect(&region.applied_pdf_point_rect)?;
    if let Some(dev) = &region.device_pixel_rect {
        validate_device_pixel_rect(dev)?;
    }
    if !region.requested_safety_margin_points.is_finite()
        || region.requested_safety_margin_points < MIN_SAFETY_MARGIN_POINTS
        || region.requested_safety_margin_points > MAX_SAFETY_MARGIN_POINTS
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "requested_safety_margin_points out of bounds",
        ));
    }

    let m = &region.applied_margins;
    if !m.left.is_finite() || !m.right.is_finite() || !m.top.is_finite() || !m.bottom.is_finite() {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Applied margin values must be finite numbers",
        ));
    }
    if m.left < 0.0 || m.right < 0.0 || m.top < 0.0 || m.bottom < 0.0 {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Applied margin values must be >= 0.0",
        ));
    }
    let req_m = region.requested_safety_margin_points;
    let eps = GEOMETRY_EPSILON;
    if m.left > req_m + eps
        || m.right > req_m + eps
        || m.top > req_m + eps
        || m.bottom > req_m + eps
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Applied margin exceeds requested safety margin beyond tolerance",
        ));
    }

    let base = &region.base_pdf_point_rect;
    let applied = &region.applied_pdf_point_rect;

    if applied.left > base.left + eps
        || applied.bottom > base.bottom + eps
        || applied.left + applied.width < base.left + base.width - eps
        || applied.bottom + applied.height < base.bottom + base.height - eps
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Applied rectangle does not fully cover base rectangle",
        ));
    }

    if (applied.left - (base.left - m.left)).abs() > eps
        || (applied.bottom - (base.bottom - m.bottom)).abs() > eps
        || ((applied.left + applied.width) - (base.left + base.width + m.right)).abs() > eps
        || ((applied.bottom + applied.height) - (base.bottom + base.height + m.top)).abs() > eps
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Applied rectangle does not match base rectangle and applied margins",
        ));
    }

    if req_m == 0.0 {
        if m.left > eps
            || m.right > eps
            || m.top > eps
            || m.bottom > eps
            || region.boundary_adjusted
        {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                "Requested safety margin is 0.0 but applied margins are non-zero or boundary_adjusted is true",
            ));
        }
    } else if !region.boundary_adjusted {
        if (m.left - req_m).abs() > eps
            || (m.right - req_m).abs() > eps
            || (m.top - req_m).abs() > eps
            || (m.bottom - req_m).abs() > eps
        {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                "boundary_adjusted is false but applied margins differ from requested margin",
            ));
        }
    } else {
        if m.left >= req_m - eps
            && m.right >= req_m - eps
            && m.top >= req_m - eps
            && m.bottom >= req_m - eps
        {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                "boundary_adjusted is true but all applied margins equal requested margin",
            ));
        }
    }

    Ok(())
}

pub fn validate_canonical_redaction_region_with_context(
    region: &CanonicalRedactionRegion,
    page_geometry: Option<&CanonicalPageGeometry>,
    policy_safety_margin: Option<f64>,
) -> Result<(), SessionJobOperationError> {
    validate_canonical_redaction_region(region)?;

    let eps = GEOMETRY_EPSILON;
    if let Some(policy_margin) = policy_safety_margin {
        if (region.requested_safety_margin_points - policy_margin).abs() > eps {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!(
                    "requested_safety_margin_points ({}) does not match policy safety margin ({})",
                    region.requested_safety_margin_points, policy_margin
                ),
            ));
        }
    }

    if let Some(geom) = page_geometry {
        if region.page_index != geom.page_index {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!(
                    "Canonical region page_index ({}) mismatch with page geometry page_index ({})",
                    region.page_index, geom.page_index
                ),
            ));
        }

        let expected_base =
            normalized_top_left_to_pdf_point_rect(&region.original_normalized_rect, geom)?;
        let base = &region.base_pdf_point_rect;
        if (base.left - expected_base.left).abs() > eps
            || (base.bottom - expected_base.bottom).abs() > eps
            || (base.width - expected_base.width).abs() > eps
            || (base.height - expected_base.height).abs() > eps
        {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                "base_pdf_point_rect does not match recomputed rect from original_normalized_rect and page geometry",
            ));
        }

        let eff = &geom.effective_box;
        if base.left < eff.left - eps
            || base.bottom < eff.bottom - eps
            || base.left + base.width > eff.left + eff.width + eps
            || base.bottom + base.height > eff.bottom + eff.height + eps
        {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                "base_pdf_point_rect extends outside effective_box",
            ));
        }

        let applied = &region.applied_pdf_point_rect;
        if applied.left < eff.left - eps
            || applied.bottom < eff.bottom - eps
            || applied.left + applied.width > eff.left + eff.width + eps
            || applied.bottom + applied.height > eff.bottom + eff.height + eps
        {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                "applied_pdf_point_rect extends outside effective_box",
            ));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 3. Policy Contract
// ---------------------------------------------------------------------------

// v2 signs all page boundaries that are reproduced by the transform worker.
// A separate version keeps the v1 wire contract from being silently widened.
pub const SECURE_REDACTION_PLAN_VERSION: &str = "secure_redaction_plan_v2";

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RedactionProfile {
    SecureRasterizedPagesV1,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RedactionStrategy {
    RasterizedPages,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PageRedactionStrategy {
    RasterizePage,
    ImportUnaffectedPage,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RedactionAppearancePolicy {
    pub kind: String,  // "solid"
    pub color: String, // "black"
    pub opacity: f64,  // 1.0
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InteractiveContentPolicy {
    Strip,
    Block,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignaturePolicy {
    Block,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EncryptionPolicy {
    Block,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SecureRedactionPolicy {
    pub schema_version: String,
    pub profile: RedactionProfile,
    pub strategy: RedactionStrategy,
    pub raster_dpi: u32,
    pub appearance: RedactionAppearancePolicy,
    pub safety_margin_points: f64,
    pub interactive_content_policy: InteractiveContentPolicy,
    pub signature_policy: SignaturePolicy,
    pub encryption_policy: EncryptionPolicy,
}

pub fn validate_secure_redaction_policy(
    policy: &SecureRedactionPolicy,
) -> Result<(), SessionJobOperationError> {
    if policy.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            format!(
                "Unsupported policy schema version: '{}'",
                policy.schema_version
            ),
        ));
    }
    if policy.profile != RedactionProfile::SecureRasterizedPagesV1 {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Unsupported redaction profile",
        ));
    }
    if policy.strategy != RedactionStrategy::RasterizedPages {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Unsupported redaction strategy",
        ));
    }
    if policy.raster_dpi < MIN_RASTER_DPI || policy.raster_dpi > MAX_RASTER_DPI {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!(
                "raster_dpi must be between {} and {}",
                MIN_RASTER_DPI, MAX_RASTER_DPI
            ),
        ));
    }
    if !policy.safety_margin_points.is_finite()
        || policy.safety_margin_points < MIN_SAFETY_MARGIN_POINTS
        || policy.safety_margin_points > MAX_SAFETY_MARGIN_POINTS
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!(
                "safety_margin_points must be a finite number between {} and {}",
                MIN_SAFETY_MARGIN_POINTS, MAX_SAFETY_MARGIN_POINTS
            ),
        ));
    }

    let app = &policy.appearance;
    if !app.opacity.is_finite() || app.kind != "solid" || app.color != "black" || app.opacity != 1.0
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "First version supports only appearance: kind='solid', color='black', opacity=1.0",
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 4. Acknowledgements Contract
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RedactionAcknowledgements {
    pub schema_version: String,
    pub rasterized_pages_accepted: bool,
    pub text_search_loss_accepted: bool,
    pub accessibility_loss_accepted: bool,
    pub annotation_loss_accepted: bool,
    pub form_loss_accepted: bool,
    pub signature_invalidation_accepted: bool,
    pub encryption_change_accepted: bool,
}

pub fn validate_redaction_acknowledgements(
    ack: &RedactionAcknowledgements,
) -> Result<(), SessionJobOperationError> {
    if ack.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            format!(
                "Unsupported acknowledgements schema version: '{}'",
                ack.schema_version
            ),
        ));
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RedactionAcknowledgementRequirements {
    pub schema_version: String,
    pub rasterized_pages_required: bool,
    pub text_search_loss_required: bool,
    pub accessibility_loss_required: bool,
    pub annotation_loss_required: bool,
    pub form_loss_required: bool,
    pub signature_invalidation_required: bool,
    pub encryption_change_required: bool,
}

pub fn validate_redaction_acknowledgement_requirements(
    req: &RedactionAcknowledgementRequirements,
) -> Result<(), SessionJobOperationError> {
    if req.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            format!(
                "Unsupported acknowledgement requirements schema version: '{}'",
                req.schema_version
            ),
        ));
    }
    Ok(())
}

pub fn derive_redaction_acknowledgement_requirements(
    document_facts: &DocumentRedactionFacts,
    document_losses: &RedactionFeatureLosses,
    page_plans: &[PageRedactionPlan],
) -> RedactionAcknowledgementRequirements {
    RedactionAcknowledgementRequirements {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        rasterized_pages_required: page_plans
            .iter()
            .any(|p| p.strategy == PageRedactionStrategy::RasterizePage),
        text_search_loss_required: document_losses.searchability_lost,
        accessibility_loss_required: document_losses.accessibility_tags_lost
            || document_losses.structure_association_lost,
        annotation_loss_required: document_losses.annotations_lost || document_losses.links_lost,
        form_loss_required: document_losses.forms_lost,
        signature_invalidation_required: document_facts.signature_dictionary
            != InspectionFactState::Absent
            || document_facts.signature_field != InspectionFactState::Absent,
        encryption_change_required: document_facts.encryption != InspectionFactState::Absent,
    }
}

// ---------------------------------------------------------------------------
// 5. Plan & Result DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct RedactionBlocker {
    pub schema_version: String,
    pub code: String,
    pub message: String,
    pub details: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct RedactionWarning {
    pub schema_version: String,
    pub code: String,
    pub message: String,
    pub details: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum VerificationCheckStatus {
    Passed,
    Failed,
    NotRun,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct RedactionVerificationCheck {
    pub schema_version: String,
    pub code: String,
    pub status: VerificationCheckStatus,
    pub message: String,
    pub details: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SecureRedactionPublicationReport {
    pub schema_version: String,
    pub publication_id: String,
    pub published_path: String,
    pub published_digest: String,
    pub commit_state: SecureRedactionCommitState,
}

/// Typed disk truth for a completed secure-redaction publication. `published`
/// means replacement bytes are present; durability confirmation is deliberately
/// separate so callers do not treat the platform fallback as silent success.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecureRedactionCommitState {
    NewCommitted,
    NewCommittedDurabilityUnconfirmed,
}

impl SecureRedactionCommitState {
    pub(crate) fn from_document_io(state: crate::document_io::CommitState) -> Self {
        match state {
            crate::document_io::CommitState::NewCommitted => Self::NewCommitted,
            crate::document_io::CommitState::NewCommittedDurabilityUnconfirmed => {
                Self::NewCommittedDurabilityUnconfirmed
            }
            other => {
                panic!("non-committed document_io state cannot build publication report: {other}")
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DocumentRedactionFacts {
    pub schema_version: String,
    pub total_pages: u32,
    #[serde(alias = "is_encrypted")]
    pub encryption: InspectionFactState,
    #[serde(alias = "is_signed", alias = "signature_dictionary_present")]
    pub signature_dictionary: InspectionFactState,
    #[serde(alias = "signature_field_present")]
    pub signature_field: InspectionFactState,
    #[serde(alias = "byte_range_present")]
    pub byte_range: InspectionFactState,
    #[serde(alias = "acroform_present", alias = "has_acroform")]
    pub acro_form: InspectionFactState,
    #[serde(alias = "xfa_present", alias = "has_xfa")]
    pub xfa: InspectionFactState,
    #[serde(alias = "annotations_present", alias = "has_annotations")]
    pub annotations: InspectionFactState,
    #[serde(alias = "optional_content_present")]
    pub optional_content: InspectionFactState,
    #[serde(alias = "embedded_files_present")]
    pub embedded_files: InspectionFactState,
    #[serde(alias = "javascript_present")]
    pub javascript: InspectionFactState,
    #[serde(alias = "incremental_revisions_present")]
    pub incremental_revisions: InspectionFactState,
    pub structure_tree: InspectionFactState,
    pub outlines: InspectionFactState,
    pub names_dictionary: InspectionFactState,
    pub document_actions: InspectionFactState,
    pub complex_document_resources: InspectionFactState,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PageRedactionFacts {
    pub schema_version: String,
    pub page_index: u32,
    #[serde(alias = "has_text", alias = "text_present")]
    pub text: InspectionFactState,
    #[serde(alias = "has_images", alias = "image_present")]
    pub images: InspectionFactState,
    #[serde(alias = "vector_path_present")]
    pub vector_paths: InspectionFactState,
    #[serde(alias = "form_xobject_present")]
    pub form_xobjects: InspectionFactState,
    #[serde(alias = "has_annotations", alias = "annotation_present")]
    pub annotations: InspectionFactState,
    pub link_annotations: InspectionFactState,
    #[serde(alias = "widget_present")]
    pub widgets: InspectionFactState,
    #[serde(alias = "transparency_present")]
    pub transparency: InspectionFactState,
    #[serde(alias = "optional_content_present")]
    pub optional_content: InspectionFactState,
    #[serde(alias = "complex_resources_present")]
    pub complex_resources: InspectionFactState,
    pub structure_association: InspectionFactState,
    pub page_actions: InspectionFactState,
    pub spot_colors: InspectionFactState,
    pub complex_color_spaces: InspectionFactState,
    pub geometry_profile: GeometryProfile,
    pub rotation: PageRotation,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RedactionFeatureLosses {
    pub schema_version: String,
    pub text_selection_lost: bool,
    #[serde(alias = "text_search_lost")]
    pub searchability_lost: bool,
    #[serde(alias = "accessibility_lost")]
    pub accessibility_tags_lost: bool,
    pub links_lost: bool,
    pub annotations_lost: bool,
    pub forms_lost: bool,
    #[serde(alias = "vector_graphics_rasterized")]
    pub vector_quality_lost: bool,
    pub color_space_fidelity_risk: bool,
    pub spot_colors_lost: bool,
    pub layers_lost: bool,
    pub structure_association_lost: bool,
    pub page_actions_lost: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PageRedactionPlan {
    pub schema_version: String,
    pub page_index: u32,
    pub strategy: PageRedactionStrategy,
    pub geometry: CanonicalPageGeometry,
    pub geometry_digest: String,
    pub canonical_regions: Vec<CanonicalRedactionRegion>,
    pub facts: PageRedactionFacts,
    pub feature_losses: RedactionFeatureLosses,
    pub blockers: Vec<RedactionBlocker>,
    pub warnings: Vec<RedactionWarning>,
    pub executable: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SecureRedactionPlan {
    pub schema_version: String,
    pub plan_version: String,
    #[serde(alias = "document_digest")]
    pub input_digest: String,
    pub content_generation: u64,
    pub plan_digest: String,
    pub policy: SecureRedactionPolicy,
    pub acknowledgements: RedactionAcknowledgements,
    pub acknowledgement_requirements: RedactionAcknowledgementRequirements,
    pub page_plans: Vec<PageRedactionPlan>,
    pub canonical_regions: Vec<CanonicalRedactionRegion>,
    pub facts: DocumentRedactionFacts,
    pub losses: RedactionFeatureLosses,
    pub blockers: Vec<RedactionBlocker>,
    pub warnings: Vec<RedactionWarning>,
    pub executable: bool,
    pub acknowledgements_satisfied: bool,
    pub ready_for_execution: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SecureRedactionTransformReport {
    pub schema_version: String,
    pub job_id: String,
    pub session_id: String,
    pub input_digest: String,
    pub plan_digest: String,
    pub output_relative_name: String,
    pub output_digest: String,
    pub output_size_bytes: u64,
    pub page_count: u32,
    pub affected_pages: Vec<u32>,
    pub unaffected_pages: Vec<u32>,
    pub raster_dpi: u32,
    pub applied_canonical_regions: Vec<CanonicalRedactionRegion>,
    pub affected_page_raster_geometry: Vec<TransformRasterPageEvidence>,
    pub page_feature_losses: Vec<TransformPageFeatureLoss>,
    pub background_policy: String,
    pub warnings: Vec<RedactionWarning>,
    pub transform_evidence: SecureRedactionTransformEvidence,
    pub status: String, // only "transformed_unverified" for this stage
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TransformRasterPageEvidence {
    pub page_index: u32,
    pub width: u32,
    pub height: u32,
    pub dpi: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TransformPageFeatureLoss {
    pub page_index: u32,
    pub feature_losses: RedactionFeatureLosses,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SecureRedactionTransformEvidence {
    pub new_document_constructed: bool,
    pub affected_pages_rasterized: bool,
    pub affected_page_original_objects_not_imported: bool,
    pub unaffected_pages_imported: bool,
    pub full_rewrite_used: bool,
    pub incremental_save_not_used: bool,
    pub opaque_white_background_policy: bool,
    pub lossless_raster_path: bool,
    pub output_digest_calculated: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SecureRedactionVerificationReport {
    pub schema_version: String,
    pub verification_version: String,
    pub job_id: String,
    pub session_id: String,
    pub plan_digest: String,
    pub transform_digest: String,
    pub is_verified: bool,
    pub verification_checks: Vec<RedactionVerificationCheck>,
    pub blockers: Vec<RedactionBlocker>,
    pub warnings: Vec<RedactionWarning>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SecureRedactionExecutionReport {
    pub schema_version: String,
    pub job_id: String,
    pub session_id: String,
    pub transform_report: SecureRedactionTransformReport,
    pub verification_report: SecureRedactionVerificationReport,
    pub published: bool,
    pub publication_report: Option<SecureRedactionPublicationReport>,
}

// ---------------------------------------------------------------------------
// 6. Strict Validation & Aggregation Helpers
// ---------------------------------------------------------------------------

pub fn truncate_utf8_to_byte_limit(s: &str, limit_bytes: usize) -> &str {
    if s.len() <= limit_bytes {
        return s;
    }
    let mut end = limit_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub fn aggregate_details_string(
    existing_details: Option<&str>,
    new_detail: Option<&str>,
) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(existing) = existing_details {
        for part in existing.split("; ") {
            let trimmed = part.trim();
            if !trimmed.is_empty() && !parts.contains(&trimmed) {
                parts.push(trimmed);
            }
        }
    }
    if let Some(new_d) = new_detail {
        for part in new_d.split("; ") {
            let trimmed = part.trim();
            if !trimmed.is_empty() && !parts.contains(&trimmed) {
                parts.push(trimmed);
            }
        }
    }
    if parts.is_empty() {
        return None;
    }
    parts.sort();

    let mut result = String::new();
    for part in parts {
        if !result.is_empty() {
            if result.len() + 2 > MAX_DETAILS_LEN {
                break;
            }
            result.push_str("; ");
        }
        if result.len() + part.len() <= MAX_DETAILS_LEN {
            result.push_str(part);
        } else {
            let remaining_budget = MAX_DETAILS_LEN - result.len();
            let truncated_part = truncate_utf8_to_byte_limit(part, remaining_budget);
            let trimmed = truncated_part.trim_end();
            if !trimmed.is_empty() {
                result.push_str(trimmed);
            }
            break;
        }
    }
    while result.ends_with(';') || result.ends_with(' ') {
        if result.ends_with("; ") {
            result.truncate(result.len() - 2);
        } else {
            result.pop();
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

pub fn push_aggregated_warning(warnings: &mut Vec<RedactionWarning>, warning: RedactionWarning) {
    if let Some(existing) = warnings.iter_mut().find(|w| w.code == warning.code) {
        existing.details =
            aggregate_details_string(existing.details.as_deref(), warning.details.as_deref());
    } else {
        let mut w = warning;
        w.details = aggregate_details_string(None, w.details.as_deref());
        warnings.push(w);
    }
}

pub fn push_aggregated_blocker(blockers: &mut Vec<RedactionBlocker>, blocker: RedactionBlocker) {
    if let Some(existing) = blockers.iter_mut().find(|b| b.code == blocker.code) {
        existing.details =
            aggregate_details_string(existing.details.as_deref(), blocker.details.as_deref());
    } else {
        let mut b = blocker;
        b.details = aggregate_details_string(None, b.details.as_deref());
        blockers.push(b);
    }
}

pub fn validate_document_redaction_facts(
    facts: &DocumentRedactionFacts,
) -> Result<(), SessionJobOperationError> {
    if facts.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            format!(
                "Unsupported DocumentRedactionFacts schema_version: '{}'",
                facts.schema_version
            ),
        ));
    }
    Ok(())
}

pub fn validate_page_redaction_facts(
    facts: &PageRedactionFacts,
    total_pages: u32,
) -> Result<(), SessionJobOperationError> {
    if facts.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            format!(
                "Unsupported PageRedactionFacts schema_version: '{}'",
                facts.schema_version
            ),
        ));
    }
    if facts.page_index >= total_pages {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageNotFound,
            format!(
                "PageRedactionFacts page_index ({}) out of bounds for document page count ({})",
                facts.page_index, total_pages
            ),
        ));
    }
    Ok(())
}

pub fn validate_redaction_feature_losses(
    losses: &RedactionFeatureLosses,
) -> Result<(), SessionJobOperationError> {
    if losses.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            format!(
                "Unsupported RedactionFeatureLosses schema_version: '{}'",
                losses.schema_version
            ),
        ));
    }
    Ok(())
}

pub fn evaluate_page_redaction_blockers(page_facts: &PageRedactionFacts) -> Vec<RedactionBlocker> {
    let mut page_blockers = Vec::new();
    if page_facts.complex_resources == InspectionFactState::Unknown
        || page_facts.optional_content == InspectionFactState::Unknown
        || page_facts.structure_association == InspectionFactState::Unknown
        || page_facts.page_actions == InspectionFactState::Unknown
        || page_facts.complex_color_spaces == InspectionFactState::Unknown
        || page_facts.spot_colors == InspectionFactState::Unknown
        || page_facts.transparency == InspectionFactState::Unknown
        || page_facts.widgets == InspectionFactState::Unknown
        || page_facts.link_annotations == InspectionFactState::Unknown
        || page_facts.form_xobjects == InspectionFactState::Unknown
        || page_facts.vector_paths == InspectionFactState::Unknown
    {
        push_aggregated_blocker(
            &mut page_blockers,
            RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_BLOCKED_DOCUMENT_FEATURE".to_string(),
                message: format!(
                    "Page {} contains critical unknown inspection fact state",
                    page_facts.page_index
                ),
                details: None,
            },
        );
    }
    page_blockers
}

pub fn has_merged_source_id_overflow(regions: &[CanonicalRedactionRegion]) -> bool {
    let n = regions.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let r1 = &regions[i].applied_pdf_point_rect;
            let r2 = &regions[j].applied_pdf_point_rect;

            let r1_right = r1.left + r1.width;
            let r1_top = r1.bottom + r1.height;
            let r2_right = r2.left + r2.width;
            let r2_top = r2.bottom + r2.height;

            let x_gap = if r2.left > r1_right {
                r2.left - r1_right
            } else if r1.left > r2_right {
                r1.left - r2_right
            } else {
                0.0
            };

            let y_gap = if r2.bottom > r1_top {
                r2.bottom - r1_top
            } else if r1.bottom > r2_top {
                r1.bottom - r2_top
            } else {
                0.0
            };

            let has_x_overlap = r1.left < r2_right && r2.left < r1_right;
            let has_y_overlap = r1.bottom < r2_top && r2.bottom < r1_top;

            let is_x_touch = (r1.left - r2_right).abs() <= GEOMETRY_EPSILON
                || (r1_right - r2.left).abs() <= GEOMETRY_EPSILON;
            let is_y_touch = (r1.bottom - r2_top).abs() <= GEOMETRY_EPSILON
                || (r1_top - r2.bottom).abs() <= GEOMETRY_EPSILON;

            let is_true_overlap = has_x_overlap && has_y_overlap;
            let is_edge_touch = (is_x_touch && has_y_overlap) || (is_y_touch && has_x_overlap);
            let is_corner_touch = is_x_touch && is_y_touch;

            let should_merge = if x_gap > 0.0 || y_gap > 0.0 {
                false
            } else if is_true_overlap {
                true
            } else if is_edge_touch {
                MERGE_EDGE_TOUCHING
            } else if is_corner_touch {
                MERGE_CORNER_TOUCHING
            } else {
                false
            };

            if should_merge {
                let mut combined_source_ids = regions[i].source_region_ids.clone();
                combined_source_ids.extend(regions[j].source_region_ids.clone());
                combined_source_ids.sort();
                combined_source_ids.dedup();
                if combined_source_ids.len() > MAX_SOURCE_REGION_IDS {
                    return true;
                }
            }
        }
    }
    false
}

pub fn validate_page_redaction_plan(
    page_plan: &PageRedactionPlan,
    total_pages: u32,
) -> Result<(), SessionJobOperationError> {
    if page_plan.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            "Unsupported PageRedactionPlan schema_version",
        ));
    }
    if page_plan.page_index >= total_pages {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageNotFound,
            format!(
                "PageRedactionPlan page_index ({}) out of bounds for document page count ({})",
                page_plan.page_index, total_pages
            ),
        ));
    }
    if page_plan.strategy != PageRedactionStrategy::RasterizePage
        && page_plan.strategy != PageRedactionStrategy::ImportUnaffectedPage
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Unsupported page redaction strategy",
        ));
    }
    validate_canonical_page_geometry(&page_plan.geometry)?;
    if page_plan.geometry.page_index != page_plan.page_index {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageGeometryMismatch,
            format!(
                "Page plan page_index ({}) mismatch with geometry page_index ({})",
                page_plan.page_index, page_plan.geometry.page_index
            ),
        ));
    }
    validate_sha256_digest(&page_plan.geometry_digest, "geometry_digest")?;
    let expected_geom_digest = compute_geometry_digest(&page_plan.geometry);
    if page_plan.geometry_digest != expected_geom_digest {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageGeometryMismatch,
            format!(
                "geometry_digest '{}' mismatch with recomputed geometry digest '{}'",
                page_plan.geometry_digest, expected_geom_digest
            ),
        ));
    }

    if page_plan.canonical_regions.len() > MAX_CANONICAL_REGIONS_PER_PAGE {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Page {} canonical_regions count ({}) exceeds maximum limit of {}",
                page_plan.page_index,
                page_plan.canonical_regions.len(),
                MAX_CANONICAL_REGIONS_PER_PAGE
            ),
        ));
    }

    // Strategy-Region Invariant (Prompt 7)
    if page_plan.strategy == PageRedactionStrategy::RasterizePage {
        if page_plan.canonical_regions.is_empty() {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                "Strategy rasterize_page requires at least 1 canonical region",
            ));
        }
    } else if page_plan.strategy == PageRedactionStrategy::ImportUnaffectedPage {
        if !page_plan.canonical_regions.is_empty() {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                "Strategy import_unaffected_page must have exactly 0 canonical regions",
            ));
        }
    }

    validate_page_redaction_facts(&page_plan.facts, total_pages)?;
    if page_plan.facts.page_index != page_plan.page_index {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageGeometryMismatch,
            format!(
                "Page plan page_index ({}) mismatch with page facts page_index ({})",
                page_plan.page_index, page_plan.facts.page_index
            ),
        ));
    }
    if page_plan.facts.rotation != page_plan.geometry.rotation {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageGeometryMismatch,
            format!(
                "Page facts rotation ({:?}) mismatch with page geometry rotation ({:?})",
                page_plan.facts.rotation, page_plan.geometry.rotation
            ),
        ));
    }

    validate_redaction_feature_losses(&page_plan.feature_losses)?;
    validate_unique_blockers(&page_plan.blockers)?;
    validate_unique_warnings(&page_plan.warnings)?;

    let mut expected_page_blockers = evaluate_page_redaction_blockers(&page_plan.facts);
    if page_plan.strategy == PageRedactionStrategy::RasterizePage {
        if let Some(blocker) = secure_redaction_rebuild_geometry_blocker(&page_plan.geometry) {
            push_aggregated_blocker(&mut expected_page_blockers, blocker);
        }
    }
    let has_source_id_overflow = has_merged_source_id_overflow(&page_plan.canonical_regions);
    if page_plan.canonical_regions.len() > MAX_CANONICAL_REGIONS_PER_PAGE || has_source_id_overflow
    {
        push_aggregated_blocker(
            &mut expected_page_blockers,
            RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_RESOURCE_LIMIT_EXCEEDED".to_string(),
                message: if has_source_id_overflow {
                    format!(
                        "Page {} merged canonical region source_region_ids count would exceed limit of {}",
                        page_plan.page_index, MAX_SOURCE_REGION_IDS
                    )
                } else {
                    format!(
                        "Page {} merged canonical regions count ({}) exceeds maximum limit of {}",
                        page_plan.page_index,
                        page_plan.canonical_regions.len(),
                        MAX_CANONICAL_REGIONS_PER_PAGE
                    )
                },
                details: None,
            },
        );
    }
    if page_plan.blockers != expected_page_blockers {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!(
                "Page {} blockers mismatch derived blockers from facts and geometry",
                page_plan.page_index
            ),
        ));
    }

    let derived_feature_losses = if page_plan.strategy == PageRedactionStrategy::RasterizePage {
        RedactionFeatureLosses {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            text_selection_lost: page_plan.facts.text.is_present(),
            searchability_lost: page_plan.facts.text.is_present(),
            accessibility_tags_lost: page_plan.facts.structure_association.is_present(),
            links_lost: page_plan.facts.link_annotations.is_present(),
            annotations_lost: page_plan.facts.annotations.is_present(),
            forms_lost: page_plan.facts.widgets.is_present(),
            vector_quality_lost: page_plan.facts.vector_paths.is_present()
                || page_plan.facts.form_xobjects.is_present(),
            color_space_fidelity_risk: page_plan.facts.complex_resources.is_present()
                || page_plan.facts.transparency.is_present()
                || page_plan.facts.complex_color_spaces.is_present(),
            spot_colors_lost: page_plan.facts.spot_colors.is_present(),
            layers_lost: page_plan.facts.optional_content.is_present(),
            structure_association_lost: page_plan.facts.structure_association.is_present(),
            page_actions_lost: page_plan.facts.page_actions.is_present(),
        }
    } else {
        RedactionFeatureLosses {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            text_selection_lost: false,
            searchability_lost: false,
            accessibility_tags_lost: false,
            links_lost: false,
            annotations_lost: false,
            forms_lost: false,
            vector_quality_lost: false,
            color_space_fidelity_risk: false,
            spot_colors_lost: false,
            layers_lost: false,
            structure_association_lost: false,
            page_actions_lost: false,
        }
    };

    if page_plan.feature_losses != derived_feature_losses {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!(
                "Page {} feature_losses does not match derived losses from page facts and strategy",
                page_plan.page_index
            ),
        ));
    }

    let mut prev_region: Option<&CanonicalRedactionRegion> = None;
    for region in &page_plan.canonical_regions {
        validate_canonical_redaction_region_with_context(region, Some(&page_plan.geometry), None)?;
        if region.page_index != page_plan.page_index {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!(
                    "Canonical region page_index ({}) mismatch with parent page plan page_index ({})",
                    region.page_index, page_plan.page_index
                ),
            ));
        }
        if let Some(prev) = prev_region {
            if compare_canonical_redaction_regions(prev, region) == std::cmp::Ordering::Greater {
                return Err(make_redaction_error(
                    SecureRedactionErrorCode::RedactionInvalidPolicy,
                    "Canonical regions must be in canonical sort order (pageIndex, bottom, left, regionId)",
                ));
            }
        }
        prev_region = Some(region);
    }

    let expected_page_executable = page_plan.blockers.is_empty();
    if page_plan.executable != expected_page_executable {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!(
                "Page {} executable ({}) does not match expected ({})",
                page_plan.page_index, page_plan.executable, expected_page_executable
            ),
        ));
    }

    Ok(())
}

pub fn validate_blocker(blocker: &RedactionBlocker) -> Result<(), SessionJobOperationError> {
    if blocker.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            "Unsupported blocker schema_version",
        ));
    }
    if blocker.code.trim().is_empty() || blocker.code.as_bytes().len() > MAX_BLOCKER_CODE_LEN {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Blocker code must be non-empty and at most 128 UTF-8 bytes",
        ));
    }
    if blocker.message.trim().is_empty()
        || blocker.message.as_bytes().len() > MAX_USER_SAFE_MESSAGE_LEN
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Blocker message must be non-empty and at most 2048 UTF-8 bytes",
        ));
    }
    if let Some(details) = &blocker.details {
        if details.as_bytes().len() > MAX_DETAILS_LEN {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                "Blocker details exceeds 4096 UTF-8 bytes",
            ));
        }
    }
    Ok(())
}

pub fn validate_warning(warning: &RedactionWarning) -> Result<(), SessionJobOperationError> {
    if warning.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            "Unsupported warning schema_version",
        ));
    }
    if warning.code.trim().is_empty() || warning.code.as_bytes().len() > MAX_WARNING_CODE_LEN {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Warning code must be non-empty and at most 128 UTF-8 bytes",
        ));
    }
    if warning.message.trim().is_empty()
        || warning.message.as_bytes().len() > MAX_USER_SAFE_MESSAGE_LEN
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Warning message must be non-empty and at most 2048 UTF-8 bytes",
        ));
    }
    if let Some(details) = &warning.details {
        if details.as_bytes().len() > MAX_DETAILS_LEN {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                "Warning details exceeds 4096 UTF-8 bytes",
            ));
        }
    }
    Ok(())
}

pub fn validate_verification_check(
    check: &RedactionVerificationCheck,
) -> Result<(), SessionJobOperationError> {
    if check.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            "Unsupported verification check schema_version",
        ));
    }
    if check.code.trim().is_empty() || check.code.as_bytes().len() > MAX_VERIFICATION_CHECK_CODE_LEN
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Verification check code must be non-empty and at most 256 UTF-8 bytes",
        ));
    }
    if check.message.trim().is_empty()
        || check.message.as_bytes().len() > MAX_VERIFICATION_CHECK_MESSAGE_LEN
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Verification check message must be non-empty and at most 2048 UTF-8 bytes",
        ));
    }
    if let Some(details) = &check.details {
        if details.as_bytes().len() > MAX_VERIFICATION_CHECK_DETAILS_LEN {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                "Verification check details exceeds 4096 UTF-8 bytes",
            ));
        }
    }
    Ok(())
}

pub fn validate_unique_blockers(
    blockers: &[RedactionBlocker],
) -> Result<(), SessionJobOperationError> {
    if blockers.len() > MAX_BLOCKERS {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Blockers count ({}) exceeds maximum limit of {}",
                blockers.len(),
                MAX_BLOCKERS
            ),
        ));
    }
    let mut seen_codes = HashSet::new();
    for b in blockers {
        validate_blocker(b)?;
        if !seen_codes.insert(&b.code) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!("Duplicate blocker code found in collection: '{}'", b.code),
            ));
        }
    }
    Ok(())
}

pub fn validate_unique_warnings(
    warnings: &[RedactionWarning],
) -> Result<(), SessionJobOperationError> {
    if warnings.len() > MAX_WARNINGS {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Warnings count ({}) exceeds maximum limit of {}",
                warnings.len(),
                MAX_WARNINGS
            ),
        ));
    }
    let mut seen_codes = HashSet::new();
    for w in warnings {
        validate_warning(w)?;
        if !seen_codes.insert(&w.code) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!("Duplicate warning code found in collection: '{}'", w.code),
            ));
        }
    }
    Ok(())
}

pub fn validate_unique_verification_checks(
    checks: &[RedactionVerificationCheck],
) -> Result<(), SessionJobOperationError> {
    if checks.len() > MAX_VERIFICATION_CHECKS {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Verification checks count ({}) exceeds limit of {}",
                checks.len(),
                MAX_VERIFICATION_CHECKS
            ),
        ));
    }
    let mut seen_codes = HashSet::new();
    for c in checks {
        validate_verification_check(c)?;
        if !seen_codes.insert(&c.code) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!(
                    "Duplicate verification check code found in collection: '{}'",
                    c.code
                ),
            ));
        }
    }
    Ok(())
}

pub fn validate_publication_report(
    report: &SecureRedactionPublicationReport,
) -> Result<(), SessionJobOperationError> {
    if report.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            "Unsupported publication report schema_version",
        ));
    }
    if report.publication_id.trim().is_empty()
        || report.publication_id.as_bytes().len() > MAX_IDENTITY_LEN
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "publication_id must be non-empty and at most 128 UTF-8 bytes",
        ));
    }
    if report.published_path.trim().is_empty()
        || report.published_path.as_bytes().len() > MAX_PATH_LEN
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "published_path must be non-empty and at most 4096 UTF-8 bytes",
        ));
    }
    validate_sha256_digest(&report.published_digest, "published_digest")?;
    if !matches!(
        report.commit_state,
        SecureRedactionCommitState::NewCommitted
            | SecureRedactionCommitState::NewCommittedDurabilityUnconfirmed
    ) {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Publication report commit_state must be a definitive committed state",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 7. Policy Helper for Document Feature Blockers
// ---------------------------------------------------------------------------

pub fn evaluate_document_blockers(
    facts: &DocumentRedactionFacts,
    policy: &SecureRedactionPolicy,
    _ack: &RedactionAcknowledgements,
    page_plans: Option<&[PageRedactionPlan]>,
) -> Vec<RedactionBlocker> {
    let mut blockers = Vec::new();
    if facts.total_pages as usize > MAX_PAGE_PLANS {
        push_aggregated_blocker(
            &mut blockers,
            RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_RESOURCE_LIMIT_EXCEEDED".to_string(),
                message: format!(
                    "Total page count ({}) exceeds maximum page plan limit of {}",
                    facts.total_pages, MAX_PAGE_PLANS
                ),
                details: None,
            },
        );
    }
    if let Some(pp_list) = page_plans {
        let mut total_estimated_pixels: u64 = 0;
        for page_plan in pp_list {
            if page_plan.strategy == PageRedactionStrategy::RasterizePage {
                let width_px = (page_plan.geometry.effective_box.width / 72.0
                    * policy.raster_dpi as f64)
                    .ceil() as u64;
                let height_px = (page_plan.geometry.effective_box.height / 72.0
                    * policy.raster_dpi as f64)
                    .ceil() as u64;
                let page_pixels = match width_px.checked_mul(height_px) {
                    Some(px) => px,
                    None => {
                        push_aggregated_blocker(
                            &mut blockers,
                            RedactionBlocker {
                                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                                code: "REDACTION_RESOURCE_LIMIT_EXCEEDED".to_string(),
                                message: format!(
                                    "Pixel count arithmetic overflow on page {}",
                                    page_plan.page_index
                                ),
                                details: None,
                            },
                        );
                        u64::MAX
                    }
                };
                if page_pixels > MAX_PIXELS_PER_PAGE {
                    push_aggregated_blocker(
                        &mut blockers,
                        RedactionBlocker {
                            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                            code: "REDACTION_RESOURCE_LIMIT_EXCEEDED".to_string(),
                            message: format!(
                                "Page {} estimated pixel count ({}) exceeds limit of {}",
                                page_plan.page_index, page_pixels, MAX_PIXELS_PER_PAGE
                            ),
                            details: None,
                        },
                    );
                }
                total_estimated_pixels = total_estimated_pixels.saturating_add(page_pixels);
            }
        }
        if total_estimated_pixels > MAX_TOTAL_ESTIMATED_PIXELS {
            push_aggregated_blocker(
                &mut blockers,
                RedactionBlocker {
                    schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                    code: "REDACTION_RESOURCE_LIMIT_EXCEEDED".to_string(),
                    message: format!(
                        "Total estimated pixels ({}) exceeds limit of {}",
                        total_estimated_pixels, MAX_TOTAL_ESTIMATED_PIXELS
                    ),
                    details: None,
                },
            );
        }
    }
    if facts.encryption == InspectionFactState::Present
        || facts.encryption == InspectionFactState::Unknown
    {
        if policy.encryption_policy == EncryptionPolicy::Block {
            push_aggregated_blocker(
                &mut blockers,
                RedactionBlocker {
                    schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                    code: "REDACTION_ENCRYPTED_INPUT_UNSUPPORTED".to_string(),
                    message: "Encrypted inputs are unsupported in current redaction profile"
                        .to_string(),
                    details: Some(
                        "Encryption change acknowledgement cannot bypass this policy blocker"
                            .to_string(),
                    ),
                },
            );
        }
    }
    if (facts.signature_dictionary == InspectionFactState::Present
        || facts.signature_field == InspectionFactState::Present
        || facts.byte_range == InspectionFactState::Present
        || facts.signature_dictionary == InspectionFactState::Unknown
        || facts.signature_field == InspectionFactState::Unknown
        || facts.byte_range == InspectionFactState::Unknown)
        && policy.signature_policy == SignaturePolicy::Block
    {
        push_aggregated_blocker(
            &mut blockers,
            RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_SIGNED_INPUT_UNSUPPORTED".to_string(),
                message: "Signed inputs are unsupported in current redaction profile".to_string(),
                details: Some(
                    "Signature invalidation acknowledgement cannot bypass this policy blocker"
                        .to_string(),
                ),
            },
        );
    }
    if facts.xfa == InspectionFactState::Present || facts.xfa == InspectionFactState::Unknown {
        push_aggregated_blocker(
            &mut blockers,
            RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_XFA_INPUT_UNSUPPORTED".to_string(),
                message: "XFA dynamic forms are unsupported in current redaction profile"
                    .to_string(),
                details: None,
            },
        );
    }
    if (facts.acro_form == InspectionFactState::Present
        || facts.acro_form == InspectionFactState::Unknown)
        && policy.interactive_content_policy == InteractiveContentPolicy::Block
    {
        push_aggregated_blocker(
            &mut blockers,
            RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_BLOCKED_DOCUMENT_FEATURE".to_string(),
                message: "Interactive AcroForm features are blocked under conservative policy"
                    .to_string(),
                details: None,
            },
        );
    }
    if facts.complex_document_resources == InspectionFactState::Present
        || facts.complex_document_resources == InspectionFactState::Unknown
    {
        push_aggregated_blocker(
            &mut blockers,
            RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_BLOCKED_DOCUMENT_FEATURE".to_string(),
                message: "Complex document resources present or unknown".to_string(),
                details: None,
            },
        );
    }
    if facts.javascript == InspectionFactState::Present
        || facts.javascript == InspectionFactState::Unknown
    {
        push_aggregated_blocker(
            &mut blockers,
            RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_BLOCKED_DOCUMENT_FEATURE".to_string(),
                message: "Document JavaScript present or unknown".to_string(),
                details: None,
            },
        );
    }
    if facts.document_actions == InspectionFactState::Present
        || facts.document_actions == InspectionFactState::Unknown
    {
        push_aggregated_blocker(
            &mut blockers,
            RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_BLOCKED_DOCUMENT_FEATURE".to_string(),
                message: "Document actions present or unknown".to_string(),
                details: None,
            },
        );
    }
    if facts.embedded_files == InspectionFactState::Present
        || facts.embedded_files == InspectionFactState::Unknown
    {
        push_aggregated_blocker(
            &mut blockers,
            RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_BLOCKED_DOCUMENT_FEATURE".to_string(),
                message: "Embedded files present or unknown".to_string(),
                details: None,
            },
        );
    }
    if facts.names_dictionary == InspectionFactState::Present
        || facts.names_dictionary == InspectionFactState::Unknown
    {
        push_aggregated_blocker(
            &mut blockers,
            RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_BLOCKED_DOCUMENT_FEATURE".to_string(),
                message: "Names dictionary present or unknown".to_string(),
                details: None,
            },
        );
    }
    blockers
}

// ---------------------------------------------------------------------------
// 8. Cross-Field Invariant Validations
// ---------------------------------------------------------------------------

pub fn validate_secure_redaction_plan(
    plan: &SecureRedactionPlan,
) -> Result<(), SessionJobOperationError> {
    let serialized_bytes = serde_json::to_vec(plan).map_err(|e| {
        make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!("Plan serialization error: {}", e),
        )
    })?;
    if serialized_bytes.len() > MAX_REPORT_SIZE_BYTES {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Serialized plan size ({} bytes) exceeds MAX_REPORT_SIZE_BYTES ({} bytes)",
                serialized_bytes.len(),
                MAX_REPORT_SIZE_BYTES
            ),
        ));
    }

    if plan.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            format!("Unsupported plan schema version: '{}'", plan.schema_version),
        ));
    }
    if plan.plan_version != SECURE_REDACTION_PLAN_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            format!("Unsupported plan_version: '{}'", plan.plan_version),
        ));
    }
    validate_sha256_digest(&plan.input_digest, "input_digest")?;
    validate_sha256_digest(&plan.plan_digest, "plan_digest")?;
    validate_secure_redaction_policy(&plan.policy)?;
    validate_redaction_acknowledgements(&plan.acknowledgements)?;
    validate_redaction_acknowledgement_requirements(&plan.acknowledgement_requirements)?;
    validate_document_redaction_facts(&plan.facts)?;
    validate_redaction_feature_losses(&plan.losses)?;

    if plan.page_plans.len() != plan.facts.total_pages as usize {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPageNotFound,
            format!(
                "Page plan count ({}) must strictly equal document total_pages ({})",
                plan.page_plans.len(),
                plan.facts.total_pages
            ),
        ));
    }

    if plan.page_plans.len() > MAX_PAGE_PLANS {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Page plans count ({}) exceeds maximum limit of {}",
                plan.page_plans.len(),
                MAX_PAGE_PLANS
            ),
        ));
    }

    for (expected_idx, page_plan) in plan.page_plans.iter().enumerate() {
        validate_page_redaction_plan(page_plan, plan.facts.total_pages)?;
        if page_plan.page_index as usize != expected_idx {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!(
                    "Page plan index ({}) is out of canonical order at position {}",
                    page_plan.page_index, expected_idx
                ),
            ));
        }
    }

    let mut expected_top_regions = Vec::new();
    for pp in &plan.page_plans {
        expected_top_regions.extend(pp.canonical_regions.clone());
    }

    if plan.canonical_regions.len() != expected_top_regions.len() {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!(
                "Top-level canonical_regions count ({}) does not match flattened page-plan canonical regions count ({})",
                plan.canonical_regions.len(),
                expected_top_regions.len()
            ),
        ));
    }

    let mut seen_canonical_ids = HashSet::new();
    for (top_r, exp_r) in plan
        .canonical_regions
        .iter()
        .zip(expected_top_regions.iter())
    {
        validate_canonical_redaction_region_with_context(
            top_r,
            plan.page_plans
                .get(top_r.page_index as usize)
                .map(|p| &p.geometry),
            Some(plan.policy.safety_margin_points),
        )?;
        if top_r != exp_r {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!(
                    "Top-level canonical region '{}' mismatch with page-level canonical region",
                    top_r.region_id
                ),
            ));
        }
        if !seen_canonical_ids.insert(&top_r.region_id) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionDuplicateRegionId,
                format!(
                    "Duplicate global canonical region ID found: '{}'",
                    top_r.region_id
                ),
            ));
        }
    }

    validate_unique_blockers(&plan.blockers)?;
    validate_unique_warnings(&plan.warnings)?;

    let mut doc_text_lost = false;
    let mut doc_search_lost = false;
    let mut doc_a11y_lost = false;
    let mut doc_links_lost = false;
    let mut doc_annot_lost = false;
    let mut doc_forms_lost = false;
    let mut doc_vector_rasterized = false;
    let mut doc_color_risk = false;
    let mut doc_spot_lost = false;
    let mut doc_layers_lost = false;
    let mut doc_struct_lost = false;
    let mut doc_actions_lost = false;

    for pp in &plan.page_plans {
        if pp.feature_losses.text_selection_lost {
            doc_text_lost = true;
        }
        if pp.feature_losses.searchability_lost {
            doc_search_lost = true;
        }
        if pp.feature_losses.accessibility_tags_lost {
            doc_a11y_lost = true;
        }
        if pp.feature_losses.links_lost {
            doc_links_lost = true;
        }
        if pp.feature_losses.annotations_lost {
            doc_annot_lost = true;
        }
        if pp.feature_losses.forms_lost {
            doc_forms_lost = true;
        }
        if pp.feature_losses.vector_quality_lost {
            doc_vector_rasterized = true;
        }
        if pp.feature_losses.color_space_fidelity_risk {
            doc_color_risk = true;
        }
        if pp.feature_losses.spot_colors_lost {
            doc_spot_lost = true;
        }
        if pp.feature_losses.layers_lost {
            doc_layers_lost = true;
        }
        if pp.feature_losses.structure_association_lost {
            doc_struct_lost = true;
        }
        if pp.feature_losses.page_actions_lost {
            doc_actions_lost = true;
        }
    }

    let expected_doc_losses = RedactionFeatureLosses {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        text_selection_lost: doc_text_lost,
        searchability_lost: doc_search_lost,
        accessibility_tags_lost: doc_a11y_lost,
        links_lost: doc_links_lost,
        annotations_lost: doc_annot_lost,
        forms_lost: doc_forms_lost,
        vector_quality_lost: doc_vector_rasterized,
        color_space_fidelity_risk: doc_color_risk,
        spot_colors_lost: doc_spot_lost,
        layers_lost: doc_layers_lost,
        structure_association_lost: doc_struct_lost,
        page_actions_lost: doc_actions_lost,
    };

    if plan.losses != expected_doc_losses {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Document losses mismatch aggregated page feature losses",
        ));
    }

    let expected_ack_reqs =
        derive_redaction_acknowledgement_requirements(&plan.facts, &plan.losses, &plan.page_plans);

    if plan.acknowledgement_requirements != expected_ack_reqs {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Document acknowledgement_requirements mismatch derived requirements",
        ));
    }

    let expected_doc_blockers = evaluate_document_blockers(
        &plan.facts,
        &plan.policy,
        &plan.acknowledgements,
        Some(&plan.page_plans),
    );
    let mut expected_top_level_blockers = expected_doc_blockers.clone();
    let mut expected_top_level_warnings = Vec::new();
    for page_plan in &plan.page_plans {
        for b in &page_plan.blockers {
            push_aggregated_blocker(&mut expected_top_level_blockers, b.clone());
        }
        for w in &page_plan.warnings {
            push_aggregated_warning(&mut expected_top_level_warnings, w.clone());
        }
    }

    if plan.blockers != expected_top_level_blockers {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Top-level blockers mismatch expected aggregated blockers",
        ));
    }

    if plan.warnings != expected_top_level_warnings {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Top-level warnings mismatch expected aggregated warnings",
        ));
    }

    let expected_executable =
        expected_doc_blockers.is_empty() && plan.page_plans.iter().all(|p| p.executable);
    if plan.executable != expected_executable {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!(
                "plan.executable ({}) mismatch expected ({})",
                plan.executable, expected_executable
            ),
        ));
    }

    let expected_acks_satisfied = evaluate_acknowledgements_satisfied(
        &plan.acknowledgement_requirements,
        &plan.acknowledgements,
    );
    if plan.acknowledgements_satisfied != expected_acks_satisfied {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!(
                "plan.acknowledgements_satisfied ({}) does not match expected ({})",
                plan.acknowledgements_satisfied, expected_acks_satisfied
            ),
        ));
    }

    let expected_ready = plan.executable && expected_acks_satisfied;
    if plan.ready_for_execution != expected_ready {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!(
                "plan.ready_for_execution ({}) does not match expected ({})",
                plan.ready_for_execution, expected_ready
            ),
        ));
    }

    let serialized_plan_bytes = serde_json::to_vec(plan)
        .map_err(|e| {
            make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!("Failed to serialize plan: {}", e),
            )
        })?
        .len();
    if serialized_plan_bytes > MAX_REPORT_SIZE_BYTES {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Plan serialized size ({} bytes) exceeds MAX_REPORT_SIZE_BYTES limit ({})",
                serialized_plan_bytes, MAX_REPORT_SIZE_BYTES
            ),
        ));
    }

    let expected_plan_digest = compute_plan_digest(
        &plan.input_digest,
        plan.content_generation,
        &plan.policy,
        &plan.acknowledgements,
        &plan.acknowledgement_requirements,
        &plan.facts,
        &plan.page_plans,
        &plan.canonical_regions,
        &plan.losses,
        &plan.blockers,
        &plan.warnings,
        plan.executable,
        plan.acknowledgements_satisfied,
        plan.ready_for_execution,
    );

    if plan.plan_digest != expected_plan_digest {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPlanDigestMismatch,
            format!(
                "plan_digest mismatch: expected '{}', got '{}'",
                expected_plan_digest, plan.plan_digest
            ),
        ));
    }

    Ok(())
}

pub fn validate_transform_report(
    report: &SecureRedactionTransformReport,
) -> Result<(), SessionJobOperationError> {
    let serialized_bytes = serde_json::to_vec(report).map_err(|error| {
        make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!("Transform report serialization failed: {error}"),
        )
    })?;
    if serialized_bytes.len() > MAX_REPORT_SIZE_BYTES {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Transform report serialized size ({}) exceeds MAX_REPORT_SIZE_BYTES ({})",
                serialized_bytes.len(),
                MAX_REPORT_SIZE_BYTES
            ),
        ));
    }
    if report.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            format!(
                "Unsupported transform report schema version: '{}'",
                report.schema_version
            ),
        ));
    }
    if report.job_id.trim().is_empty() || report.job_id.as_bytes().len() > MAX_IDENTITY_LEN {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "job_id invalid in transform report",
        ));
    }
    if report.session_id.trim().is_empty() || report.session_id.as_bytes().len() > MAX_IDENTITY_LEN
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "session_id invalid in transform report",
        ));
    }
    validate_sha256_digest(&report.plan_digest, "plan_digest")?;
    validate_sha256_digest(&report.input_digest, "input_digest")?;
    validate_sha256_digest(&report.output_digest, "output_digest")?;
    if report.output_relative_name != "secure-redaction-transform.pdf" {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "output_relative_name must be the fixed internal artifact name",
        ));
    }
    if report.output_size_bytes == 0 || report.output_size_bytes > MAX_SECURE_REDACTION_OUTPUT_BYTES
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "output_size_bytes ({}) is outside 1..={} bytes",
                report.output_size_bytes, MAX_SECURE_REDACTION_OUTPUT_BYTES
            ),
        ));
    }
    if report.status != "transformed_unverified" {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Transform report status must be 'transformed_unverified'",
        ));
    }
    if report.page_count as usize > MAX_PAGE_PLANS
        || report.applied_canonical_regions.len() > MAX_REGIONS_PER_INPUT
        || report.affected_page_raster_geometry.len() > MAX_PAGE_PLANS
        || report.page_feature_losses.len() > MAX_PAGE_PLANS
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            "Transform report collections exceed their bounds",
        ));
    }
    if report.raster_dpi < MIN_RASTER_DPI || report.raster_dpi > MAX_RASTER_DPI {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!(
                "Transform report raster_dpi must be between {MIN_RASTER_DPI} and {MAX_RASTER_DPI}"
            ),
        ));
    }
    if report.background_policy != "opaque_white_v1" {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Transform report background_policy must be opaque_white_v1",
        ));
    }

    let mut affected = HashSet::new();
    for &page_index in &report.affected_pages {
        if page_index >= report.page_count {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageNotFound,
                format!("Affected page index {page_index} is outside page_count"),
            ));
        }
        if !affected.insert(page_index) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!("Duplicate affected page index {page_index}"),
            ));
        }
    }
    let mut unaffected = HashSet::new();
    for &page_index in &report.unaffected_pages {
        if page_index >= report.page_count {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageNotFound,
                format!("Unaffected page index {page_index} is outside page_count"),
            ));
        }
        if !unaffected.insert(page_index) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!("Duplicate unaffected page index {page_index}"),
            ));
        }
        if affected.contains(&page_index) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!("Page index {page_index} is both affected and unaffected"),
            ));
        }
    }
    if affected.len() + unaffected.len() != report.page_count as usize {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Affected and unaffected pages must be a complete page partition",
        ));
    }

    if report.affected_page_raster_geometry.len() != affected.len() {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Affected raster evidence must contain exactly one entry per affected page",
        ));
    }
    let mut raster_pages = HashSet::new();
    for raster in &report.affected_page_raster_geometry {
        if !affected.contains(&raster.page_index) || !raster_pages.insert(raster.page_index) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!(
                    "Raster evidence page {} is duplicate or not affected",
                    raster.page_index
                ),
            ));
        }
        if raster.width == 0
            || raster.height == 0
            || raster.width > MAX_RASTER_DIMENSION
            || raster.height > MAX_RASTER_DIMENSION
            || raster.dpi < MIN_RASTER_DPI
            || raster.dpi > MAX_RASTER_DPI
            || raster.dpi != report.raster_dpi
        {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!(
                    "Raster evidence for page {} has invalid dimensions or DPI",
                    raster.page_index
                ),
            ));
        }
        let pixels = (raster.width as u64)
            .checked_mul(raster.height as u64)
            .ok_or_else(|| {
                make_redaction_error(
                    SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                    "Raster evidence pixel-count arithmetic overflow",
                )
            })?;
        if pixels > MAX_PIXELS_PER_PAGE {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionResourceLimitExceeded,
                format!(
                    "Raster evidence for page {} exceeds pixel limit",
                    raster.page_index
                ),
            ));
        }
    }

    if report.page_feature_losses.len() != report.page_count as usize {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Feature-loss evidence must contain exactly one entry per page",
        ));
    }
    let mut loss_pages = HashSet::new();
    for loss in &report.page_feature_losses {
        if loss.page_index >= report.page_count || !loss_pages.insert(loss.page_index) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!(
                    "Feature-loss evidence page {} is duplicate or out of range",
                    loss.page_index
                ),
            ));
        }
        validate_redaction_feature_losses(&loss.feature_losses)?;
    }

    let mut region_ids = HashSet::new();
    let mut source_region_ids = HashSet::new();
    for region in &report.applied_canonical_regions {
        validate_canonical_redaction_region(region)?;
        if !affected.contains(&region.page_index) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!(
                    "Applied canonical region '{}' is not on an affected page",
                    region.region_id
                ),
            ));
        }
        if !region_ids.insert(&region.region_id) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionDuplicateRegionId,
                format!(
                    "Duplicate applied canonical region ID '{}'",
                    region.region_id
                ),
            ));
        }
        for source_region_id in &region.source_region_ids {
            if !source_region_ids.insert(source_region_id) {
                return Err(make_redaction_error(
                    SecureRedactionErrorCode::RedactionDuplicateRegionId,
                    format!("Duplicate applied source region ID '{source_region_id}'"),
                ));
            }
        }
    }

    let evidence = &report.transform_evidence;
    if !evidence.new_document_constructed
        || !evidence.full_rewrite_used
        || !evidence.incremental_save_not_used
        || !evidence.opaque_white_background_policy
        || !evidence.lossless_raster_path
        || !evidence.output_digest_calculated
        || (!affected.is_empty()
            && (!evidence.affected_pages_rasterized
                || !evidence.affected_page_original_objects_not_imported))
        || (!unaffected.is_empty() && !evidence.unaffected_pages_imported)
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Transform evidence contradicts the reported transform collections or required path",
        ));
    }
    validate_unique_warnings(&report.warnings)?;
    Ok(())
}

pub fn validate_verification_report(
    report: &SecureRedactionVerificationReport,
) -> Result<(), SessionJobOperationError> {
    if report.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            format!(
                "Unsupported verification report schema version: '{}'",
                report.schema_version
            ),
        ));
    }
    if report.verification_version != SECURE_REDACTION_VERIFICATION_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            "Unsupported secure redaction verification version",
        ));
    }
    if report.job_id.trim().is_empty() || report.job_id.as_bytes().len() > MAX_IDENTITY_LEN {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "job_id invalid in verification report",
        ));
    }
    if report.session_id.trim().is_empty() || report.session_id.as_bytes().len() > MAX_IDENTITY_LEN
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "session_id invalid in verification report",
        ));
    }
    validate_sha256_digest(&report.plan_digest, "plan_digest")?;
    validate_sha256_digest(&report.transform_digest, "transform_digest")?;

    validate_unique_verification_checks(&report.verification_checks)?;
    validate_unique_blockers(&report.blockers)?;
    validate_unique_warnings(&report.warnings)?;

    if report.verification_checks.len() != REQUIRED_SECURE_REDACTION_VERIFICATION_CHECKS.len()
        || report
            .verification_checks
            .iter()
            .map(|check| check.code.as_str())
            .ne(REQUIRED_SECURE_REDACTION_VERIFICATION_CHECKS
                .iter()
                .copied())
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Verification report must contain the exact canonical required check set in order",
        ));
    }

    let expected_verified = report.blockers.is_empty()
        && report
            .verification_checks
            .iter()
            .all(|check| check.status == VerificationCheckStatus::Passed);
    if report.is_verified != expected_verified {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Verification report is_verified must equal (all required checks passed AND blockers empty)",
        ));
    }

    Ok(())
}

pub fn validate_execution_report(
    report: &SecureRedactionExecutionReport,
) -> Result<(), SessionJobOperationError> {
    if report.schema_version != REDACTION_SCHEMA_VERSION {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPolicyVersionUnsupported,
            format!(
                "Unsupported execution report schema version: '{}'",
                report.schema_version
            ),
        ));
    }
    if report.job_id.trim().is_empty() || report.job_id.as_bytes().len() > MAX_IDENTITY_LEN {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "job_id invalid in execution report",
        ));
    }
    if report.session_id.trim().is_empty() || report.session_id.as_bytes().len() > MAX_IDENTITY_LEN
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "session_id invalid in execution report",
        ));
    }

    validate_transform_report(&report.transform_report)?;
    validate_verification_report(&report.verification_report)?;

    if report.job_id != report.transform_report.job_id
        || report.job_id != report.verification_report.job_id
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Cross-report mismatch: job_id does not match across execution, transform, and verification reports",
        ));
    }
    if report.session_id != report.transform_report.session_id
        || report.session_id != report.verification_report.session_id
    {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            "Cross-report mismatch: session_id does not match across execution, transform, and verification reports",
        ));
    }
    if report.verification_report.plan_digest != report.transform_report.plan_digest {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPlanDigestMismatch,
            "Cross-report mismatch: verification_report.plan_digest does not match transform_report.plan_digest",
        ));
    }
    if report.verification_report.transform_digest != report.transform_report.output_digest {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionPlanDigestMismatch,
            "Cross-report mismatch: verification_report.transform_digest does not match transform_report.output_digest",
        ));
    }

    if report.published {
        let pub_report = match &report.publication_report {
            Some(p) => p,
            None => {
                return Err(make_redaction_error(
                    SecureRedactionErrorCode::RedactionInvalidPolicy,
                    "Execution report published is true but publication_report is None",
                ))
            }
        };
        validate_publication_report(pub_report)?;

        if pub_report.published_digest != report.transform_report.output_digest {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPlanDigestMismatch,
                "Publication mismatch: published_digest does not match transform_report.output_digest",
            ));
        }
        if !report.verification_report.is_verified {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                "Execution report cannot be published if verification_report.is_verified is false",
            ));
        }
        if !report.verification_report.blockers.is_empty() {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                "Execution report cannot be published if verification_report has blockers",
            ));
        }
    } else {
        if report.publication_report.is_some() {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                "Execution report published is false so publication_report must be None",
            ));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 9. Pure Redaction Planning Core (Prompt 8B-3.1)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RedactionPlannerInput {
    pub input_digest: String,
    pub content_generation: u64,
    pub policy: SecureRedactionPolicy,
    pub acknowledgements: RedactionAcknowledgements,
    pub region_inputs: Vec<RedactionRegionInput>,
    pub page_geometries: Vec<CanonicalPageGeometry>,
    pub document_facts: DocumentRedactionFacts,
    pub page_facts: Vec<PageRedactionFacts>,
}

pub fn validate_redaction_planner_input(
    input: &RedactionPlannerInput,
) -> Result<(), SessionJobOperationError> {
    validate_sha256_digest(&input.input_digest, "input_digest")?;
    validate_secure_redaction_policy(&input.policy)?;
    validate_redaction_acknowledgements(&input.acknowledgements)?;
    validate_document_redaction_facts(&input.document_facts)?;

    let mut seen_region_ids = HashSet::new();
    for region in &input.region_inputs {
        validate_redaction_region_input(region)?;
        if !seen_region_ids.insert(&region.id) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionDuplicateRegionId,
                format!("Duplicate region ID found: '{}'", region.id),
            ));
        }
    }

    let mut seen_pages = HashSet::new();
    for geom in &input.page_geometries {
        validate_canonical_page_geometry(geom)?;
        if !seen_pages.insert(geom.page_index) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!(
                    "Duplicate page index in page_geometries: {}",
                    geom.page_index
                ),
            ));
        }
        if geom.page_index >= input.document_facts.total_pages {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageNotFound,
                format!(
                    "Geometry page_index ({}) exceeds document total_pages ({})",
                    geom.page_index, input.document_facts.total_pages
                ),
            ));
        }
    }

    let mut seen_fact_pages = HashSet::new();
    for pf in &input.page_facts {
        validate_page_redaction_facts(pf, input.document_facts.total_pages)?;
        if !seen_fact_pages.insert(pf.page_index) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionInvalidPolicy,
                format!("Duplicate page index in page_facts: {}", pf.page_index),
            ));
        }
    }

    for page_idx in 0..input.document_facts.total_pages {
        if !seen_pages.contains(&page_idx) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageNotFound,
                format!("Missing CanonicalPageGeometry for page {}", page_idx),
            ));
        }
        if !seen_fact_pages.contains(&page_idx) {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageNotFound,
                format!("Missing PageRedactionFacts for page {}", page_idx),
            ));
        }
    }

    for region in &input.region_inputs {
        if region.page_index >= input.document_facts.total_pages {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageNotFound,
                format!(
                    "Region page_index ({}) exceeds document total_pages ({})",
                    region.page_index, input.document_facts.total_pages
                ),
            ));
        }
    }

    Ok(())
}

pub fn normalized_top_left_to_pdf_point_rect(
    norm: &NormalizedTopLeftRect,
    geom: &CanonicalPageGeometry,
) -> Result<PdfPointRect, SessionJobOperationError> {
    let box_eff = &geom.effective_box;
    let (nx, ny, nw, nh) = (norm.x, norm.y, norm.width, norm.height);

    let page_top = box_eff.bottom + box_eff.height;
    let pdf_left = box_eff.left + nx * box_eff.width;
    let pdf_bottom = page_top - (ny + nh) * box_eff.height;
    let pdf_width = nw * box_eff.width;
    let pdf_height = nh * box_eff.height;

    let res = PdfPointRect {
        left: pdf_left,
        bottom: pdf_bottom,
        width: pdf_width,
        height: pdf_height,
    };
    validate_pdf_point_rect(&res)?;
    Ok(res)
}

pub fn apply_safety_margin_and_clamp(
    rect: &PdfPointRect,
    margin_pts: f64,
    effective_box: &PdfPointRect,
) -> (PdfPointRect, PdfPointRect, AppliedMargins, bool) {
    let expanded_left = rect.left - margin_pts;
    let expanded_bottom = rect.bottom - margin_pts;
    let expanded_width = rect.width + 2.0 * margin_pts;
    let expanded_height = rect.height + 2.0 * margin_pts;

    let page_right = effective_box.left + effective_box.width;
    let page_top = effective_box.bottom + effective_box.height;

    let clamped_left = expanded_left.max(effective_box.left);
    let clamped_bottom = expanded_bottom.max(effective_box.bottom);
    let clamped_right = (expanded_left + expanded_width).min(page_right);
    let clamped_top = (expanded_bottom + expanded_height).min(page_top);

    let clamped_width = (clamped_right - clamped_left).max(0.0);
    let clamped_height = (clamped_top - clamped_bottom).max(0.0);

    let was_clamped = (clamped_left != expanded_left)
        || (clamped_bottom != expanded_bottom)
        || (clamped_right != expanded_left + expanded_width)
        || (clamped_top != expanded_bottom + expanded_height);

    let applied_margins = AppliedMargins {
        left: (rect.left - clamped_left).max(0.0),
        right: (clamped_right - (rect.left + rect.width)).max(0.0),
        bottom: (rect.bottom - clamped_bottom).max(0.0),
        top: (clamped_top - (rect.bottom + rect.height)).max(0.0),
    };

    (
        rect.clone(),
        PdfPointRect {
            left: clamped_left,
            bottom: clamped_bottom,
            width: clamped_width,
            height: clamped_height,
        },
        applied_margins,
        was_clamped,
    )
}

pub fn evaluate_acknowledgements_satisfied(
    reqs: &RedactionAcknowledgementRequirements,
    acks: &RedactionAcknowledgements,
) -> bool {
    (!reqs.rasterized_pages_required || acks.rasterized_pages_accepted)
        && (!reqs.text_search_loss_required || acks.text_search_loss_accepted)
        && (!reqs.accessibility_loss_required || acks.accessibility_loss_accepted)
        && (!reqs.annotation_loss_required || acks.annotation_loss_accepted)
        && (!reqs.form_loss_required || acks.form_loss_accepted)
        && (!reqs.signature_invalidation_required || acks.signature_invalidation_accepted)
        && (!reqs.encryption_change_required || acks.encryption_change_accepted)
}

#[derive(Debug, Clone)]
pub struct PageMergeOutcome {
    pub merged_regions: Vec<CanonicalRedactionRegion>,
    pub resource_limit_exceeded: bool,
}

pub fn merge_canonical_regions_on_page(
    mut regions: Vec<CanonicalRedactionRegion>,
    page_index: u32,
) -> Result<PageMergeOutcome, SessionJobOperationError> {
    let mut resource_limit_exceeded = false;
    if regions.len() <= 1 {
        for (idx, r) in regions.iter_mut().enumerate() {
            if r.source_region_ids.len() == 1 {
                r.region_id = format!("canon_p{}_{}", page_index, r.source_region_ids[0]);
            } else {
                r.region_id = format!("merged_p{}_{}", page_index, idx + 1);
            }
        }
        return Ok(PageMergeOutcome {
            merged_regions: regions,
            resource_limit_exceeded: false,
        });
    }

    regions.sort_by(compare_canonical_redaction_regions);

    let mut merged = true;
    while merged {
        merged = false;
        let mut i = 0;
        while i < regions.len() {
            let mut j = i + 1;
            while j < regions.len() {
                let r1 = &regions[i].applied_pdf_point_rect;
                let r2 = &regions[j].applied_pdf_point_rect;

                let r1_right = r1.left + r1.width;
                let r1_top = r1.bottom + r1.height;
                let r2_right = r2.left + r2.width;
                let r2_top = r2.bottom + r2.height;

                let x_gap = if r2.left > r1_right {
                    r2.left - r1_right
                } else if r1.left > r2_right {
                    r1.left - r2_right
                } else {
                    0.0
                };

                let y_gap = if r2.bottom > r1_top {
                    r2.bottom - r1_top
                } else if r1.bottom > r2_top {
                    r1.bottom - r2_top
                } else {
                    0.0
                };

                let has_x_overlap = r1.left < r2_right && r2.left < r1_right;
                let has_y_overlap = r1.bottom < r2_top && r2.bottom < r1_top;

                let is_x_touch = (r1.left - r2_right).abs() < f64::EPSILON
                    || (r1_right - r2.left).abs() < f64::EPSILON;
                let is_y_touch = (r1.bottom - r2_top).abs() < f64::EPSILON
                    || (r1_top - r2.bottom).abs() < f64::EPSILON;

                let is_true_overlap = has_x_overlap && has_y_overlap;
                let is_edge_touch = (is_x_touch && has_y_overlap) || (is_y_touch && has_x_overlap);
                let is_corner_touch = is_x_touch && is_y_touch;

                let should_merge = if x_gap > 0.0 || y_gap > 0.0 {
                    false
                } else if is_true_overlap {
                    true
                } else if is_edge_touch {
                    MERGE_EDGE_TOUCHING
                } else if is_corner_touch {
                    MERGE_CORNER_TOUCHING
                } else {
                    false
                };

                if should_merge {
                    let mut combined_source_ids = regions[i].source_region_ids.clone();
                    combined_source_ids.extend(regions[j].source_region_ids.clone());
                    combined_source_ids.sort();
                    combined_source_ids.dedup();

                    if combined_source_ids.len() > MAX_SOURCE_REGION_IDS {
                        resource_limit_exceeded = true;
                        j += 1;
                        continue;
                    }

                    let new_left = r1.left.min(r2.left);
                    let new_bottom = r1.bottom.min(r2.bottom);
                    let new_right = r1_right.max(r2_right);
                    let new_top = r1_top.max(r2_top);

                    let new_applied_rect = PdfPointRect {
                        left: new_left,
                        bottom: new_bottom,
                        width: new_right - new_left,
                        height: new_top - new_bottom,
                    };

                    let combined_orig_rect = NormalizedTopLeftRect {
                        x: regions[i]
                            .original_normalized_rect
                            .x
                            .min(regions[j].original_normalized_rect.x),
                        y: regions[i]
                            .original_normalized_rect
                            .y
                            .min(regions[j].original_normalized_rect.y),
                        width: (regions[i].original_normalized_rect.x
                            + regions[i].original_normalized_rect.width)
                            .max(
                                regions[j].original_normalized_rect.x
                                    + regions[j].original_normalized_rect.width,
                            )
                            - regions[i]
                                .original_normalized_rect
                                .x
                                .min(regions[j].original_normalized_rect.x),
                        height: (regions[i].original_normalized_rect.y
                            + regions[i].original_normalized_rect.height)
                            .max(
                                regions[j].original_normalized_rect.y
                                    + regions[j].original_normalized_rect.height,
                            )
                            - regions[i]
                                .original_normalized_rect
                                .y
                                .min(regions[j].original_normalized_rect.y),
                    };

                    let b1 = &regions[i].base_pdf_point_rect;
                    let b2 = &regions[j].base_pdf_point_rect;
                    let combined_base_rect = PdfPointRect {
                        left: b1.left.min(b2.left),
                        bottom: b1.bottom.min(b2.bottom),
                        width: (b1.left + b1.width).max(b2.left + b2.width) - b1.left.min(b2.left),
                        height: (b1.bottom + b1.height).max(b2.bottom + b2.height)
                            - b1.bottom.min(b2.bottom),
                    };

                    let margin = regions[i]
                        .requested_safety_margin_points
                        .max(regions[j].requested_safety_margin_points);

                    let combined_applied_margins = AppliedMargins {
                        left: (combined_base_rect.left - new_applied_rect.left).max(0.0),
                        right: ((new_applied_rect.left + new_applied_rect.width)
                            - (combined_base_rect.left + combined_base_rect.width))
                            .max(0.0),
                        bottom: (combined_base_rect.bottom - new_applied_rect.bottom).max(0.0),
                        top: ((new_applied_rect.bottom + new_applied_rect.height)
                            - (combined_base_rect.bottom + combined_base_rect.height))
                            .max(0.0),
                    };

                    let combined_boundary_adjusted = if margin == 0.0 {
                        false
                    } else {
                        combined_applied_margins.left < margin - GEOMETRY_EPSILON
                            || combined_applied_margins.right < margin - GEOMETRY_EPSILON
                            || combined_applied_margins.top < margin - GEOMETRY_EPSILON
                            || combined_applied_margins.bottom < margin - GEOMETRY_EPSILON
                    };

                    regions[i] = CanonicalRedactionRegion {
                        region_id: regions[i].region_id.clone(),
                        source_region_ids: combined_source_ids,
                        page_index,
                        original_normalized_rect: combined_orig_rect,
                        base_pdf_point_rect: combined_base_rect,
                        requested_safety_margin_points: margin,
                        applied_pdf_point_rect: new_applied_rect,
                        applied_margins: combined_applied_margins,
                        boundary_adjusted: combined_boundary_adjusted,
                        device_pixel_rect: None,
                    };

                    regions.remove(j);
                    merged = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
    }

    regions.sort_by(|a, b| {
        a.applied_pdf_point_rect
            .bottom
            .partial_cmp(&b.applied_pdf_point_rect.bottom)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.applied_pdf_point_rect
                    .left
                    .partial_cmp(&b.applied_pdf_point_rect.left)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.source_region_ids.cmp(&b.source_region_ids))
    });

    for (idx, r) in regions.iter_mut().enumerate() {
        if r.source_region_ids.len() == 1 {
            r.region_id = format!("canon_p{}_{}", page_index, r.source_region_ids[0]);
        } else {
            r.region_id = format!("merged_p{}_{}", page_index, idx + 1);
        }
    }

    regions.sort_by(compare_canonical_redaction_regions);

    Ok(PageMergeOutcome {
        merged_regions: regions,
        resource_limit_exceeded,
    })
}

// ---------------------------------------------------------------------------
// Canonical Serializer for Cross-Language SHA-256 Digest Parity
// ---------------------------------------------------------------------------

pub struct CanonicalSerializer {
    bytes: Vec<u8>,
}

impl CanonicalSerializer {
    pub fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub fn write_tag(&mut self, tag: u8) {
        self.bytes.push(tag);
    }

    pub fn write_u32_be(&mut self, val: u32) {
        self.bytes.extend_from_slice(&val.to_be_bytes());
    }

    pub fn write_u64_be(&mut self, val: u64) {
        self.bytes.extend_from_slice(&val.to_be_bytes());
    }

    pub fn write_string(&mut self, s: &str) {
        self.write_tag(0x01);
        let b = s.as_bytes();
        self.write_u32_be(b.len() as u32);
        self.bytes.extend_from_slice(b);
    }

    pub fn write_bool(&mut self, b: bool) {
        self.write_tag(0x02);
        self.write_u32_be(1);
        self.bytes.push(if b { 1 } else { 0 });
    }

    pub fn write_u32(&mut self, val: u32) {
        self.write_tag(0x03);
        self.write_u32_be(4);
        self.write_u32_be(val);
    }

    pub fn write_i32(&mut self, val: i32) {
        self.write_tag(0x0A);
        self.write_u32_be(4);
        self.bytes.extend_from_slice(&val.to_be_bytes());
    }

    pub fn write_u64(&mut self, val: u64) {
        self.write_tag(0x04);
        self.write_u32_be(8);
        self.write_u64_be(val);
    }

    pub fn write_f64(&mut self, val: f64) {
        self.write_tag(0x05);
        self.write_u32_be(8);
        let norm_val = if val == 0.0 { 0.0 } else { val };
        self.bytes.extend_from_slice(&norm_val.to_be_bytes());
    }

    pub fn write_null(&mut self) {
        self.write_tag(0x06);
        self.write_u32_be(0);
    }

    pub fn write_field(&mut self, name: &str, f: impl FnOnce(&mut Self)) {
        self.write_tag(0x09);
        let len_pos = self.bytes.len();
        self.write_u32_be(0);
        let start_pos = self.bytes.len();

        self.write_string(name);
        f(self);

        let field_len = (self.bytes.len() - start_pos) as u32;
        self.bytes[len_pos..len_pos + 4].copy_from_slice(&field_len.to_be_bytes());
    }

    pub fn write_list_start(&mut self, count: usize) {
        self.write_tag(0x07);
        self.write_u32_be(count as u32);
    }

    pub fn write_object_start(&mut self, field_count: usize) {
        self.write_tag(0x08);
        self.write_u32_be(field_count as u32);
    }

    pub fn get_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn finalize_digest(self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(&self.bytes);
        format!("{:x}", hasher.finalize())
    }
}

fn fact_state_str(state: InspectionFactState) -> &'static str {
    match state {
        InspectionFactState::Present => "present",
        InspectionFactState::Absent => "absent",
        InspectionFactState::Unknown => "unknown",
    }
}

pub fn write_canonical_page_geometry_fields(
    s: &mut CanonicalSerializer,
    geom: &CanonicalPageGeometry,
) {
    s.write_object_start(10);
    s.write_field("schema_version", |s| {
        s.write_string(REDACTION_SCHEMA_VERSION)
    });
    s.write_field("page_index", |s| s.write_u32(geom.page_index));
    s.write_field("media_box", |s| {
        s.write_object_start(4);
        s.write_field("left", |b| b.write_f64(geom.media_box.left));
        s.write_field("bottom", |b| b.write_f64(geom.media_box.bottom));
        s.write_field("width", |b| b.write_f64(geom.media_box.width));
        s.write_field("height", |b| b.write_f64(geom.media_box.height));
    });
    s.write_field("crop_box", |s| {
        if let Some(crop) = &geom.crop_box {
            s.write_object_start(4);
            s.write_field("left", |b| b.write_f64(crop.left));
            s.write_field("bottom", |b| b.write_f64(crop.bottom));
            s.write_field("width", |b| b.write_f64(crop.width));
            s.write_field("height", |b| b.write_f64(crop.height));
        } else {
            s.write_null();
        }
    });
    for (field, boundary) in [
        ("trim_box", geom.trim_box.as_ref()),
        ("bleed_box", geom.bleed_box.as_ref()),
        ("art_box", geom.art_box.as_ref()),
    ] {
        s.write_field(field, |s| {
            if let Some(boundary) = boundary {
                s.write_object_start(4);
                s.write_field("left", |b| b.write_f64(boundary.left));
                s.write_field("bottom", |b| b.write_f64(boundary.bottom));
                s.write_field("width", |b| b.write_f64(boundary.width));
                s.write_field("height", |b| b.write_f64(boundary.height));
            } else {
                s.write_null();
            }
        });
    }
    s.write_field("effective_box", |s| {
        s.write_object_start(4);
        s.write_field("left", |b| b.write_f64(geom.effective_box.left));
        s.write_field("bottom", |b| b.write_f64(geom.effective_box.bottom));
        s.write_field("width", |b| b.write_f64(geom.effective_box.width));
        s.write_field("height", |b| b.write_f64(geom.effective_box.height));
    });
    s.write_field("rotation", |s| s.write_u32(geom.rotation.as_u16() as u32));
    s.write_field("geometry_profile", |s| {
        s.write_string("standard_effective_box_v1")
    });
}

pub fn compute_geometry_digest(geom: &CanonicalPageGeometry) -> String {
    let mut serializer = CanonicalSerializer::new();
    write_canonical_page_geometry_fields(&mut serializer, geom);
    serializer.finalize_digest()
}

pub fn write_canonical_region(s: &mut CanonicalSerializer, r: &CanonicalRedactionRegion) {
    s.write_object_start(10);
    s.write_field("region_id", |s| s.write_string(&r.region_id));
    s.write_field("source_region_ids", |s| {
        s.write_list_start(r.source_region_ids.len());
        for id in &r.source_region_ids {
            s.write_string(id);
        }
    });
    s.write_field("page_index", |s| s.write_u32(r.page_index));
    s.write_field("original_normalized_rect", |s| {
        s.write_object_start(4);
        s.write_field("x", |s| s.write_f64(r.original_normalized_rect.x));
        s.write_field("y", |s| s.write_f64(r.original_normalized_rect.y));
        s.write_field("width", |s| s.write_f64(r.original_normalized_rect.width));
        s.write_field("height", |s| s.write_f64(r.original_normalized_rect.height));
    });
    s.write_field("base_pdf_point_rect", |s| {
        s.write_object_start(4);
        s.write_field("left", |s| s.write_f64(r.base_pdf_point_rect.left));
        s.write_field("bottom", |s| s.write_f64(r.base_pdf_point_rect.bottom));
        s.write_field("width", |s| s.write_f64(r.base_pdf_point_rect.width));
        s.write_field("height", |s| s.write_f64(r.base_pdf_point_rect.height));
    });
    s.write_field("requested_safety_margin_points", |s| {
        s.write_f64(r.requested_safety_margin_points)
    });
    s.write_field("applied_pdf_point_rect", |s| {
        s.write_object_start(4);
        s.write_field("left", |s| s.write_f64(r.applied_pdf_point_rect.left));
        s.write_field("bottom", |s| s.write_f64(r.applied_pdf_point_rect.bottom));
        s.write_field("width", |s| s.write_f64(r.applied_pdf_point_rect.width));
        s.write_field("height", |s| s.write_f64(r.applied_pdf_point_rect.height));
    });
    s.write_field("applied_margins", |s| {
        s.write_object_start(4);
        s.write_field("left", |s| s.write_f64(r.applied_margins.left));
        s.write_field("right", |s| s.write_f64(r.applied_margins.right));
        s.write_field("top", |s| s.write_f64(r.applied_margins.top));
        s.write_field("bottom", |s| s.write_f64(r.applied_margins.bottom));
    });
    s.write_field("boundary_adjusted", |s| s.write_bool(r.boundary_adjusted));
    s.write_field("device_pixel_rect", |s| {
        if let Some(dev) = &r.device_pixel_rect {
            s.write_object_start(4);
            s.write_field("left", |s| s.write_i32(dev.left));
            s.write_field("top", |s| s.write_i32(dev.top));
            s.write_field("width", |s| s.write_u32(dev.width));
            s.write_field("height", |s| s.write_u32(dev.height));
        } else {
            s.write_null();
        }
    });
}

fn write_blocker(s: &mut CanonicalSerializer, b: &RedactionBlocker) {
    s.write_object_start(4);
    s.write_field("schema_version", |s| s.write_string(&b.schema_version));
    s.write_field("code", |s| s.write_string(&b.code));
    s.write_field("message", |s| s.write_string(&b.message));
    s.write_field("details", |s| {
        if let Some(d) = &b.details {
            s.write_string(d);
        } else {
            s.write_null();
        }
    });
}

fn write_warning(s: &mut CanonicalSerializer, w: &RedactionWarning) {
    s.write_object_start(4);
    s.write_field("schema_version", |s| s.write_string(&w.schema_version));
    s.write_field("code", |s| s.write_string(&w.code));
    s.write_field("message", |s| s.write_string(&w.message));
    s.write_field("details", |s| {
        if let Some(d) = &w.details {
            s.write_string(d);
        } else {
            s.write_null();
        }
    });
}

fn write_page_redaction_facts(s: &mut CanonicalSerializer, f: &PageRedactionFacts) {
    s.write_object_start(18);
    s.write_field("schema_version", |s| s.write_string(&f.schema_version));
    s.write_field("page_index", |s| s.write_u32(f.page_index));
    s.write_field("text", |s| s.write_string(fact_state_str(f.text)));
    s.write_field("images", |s| s.write_string(fact_state_str(f.images)));
    s.write_field("vector_paths", |s| {
        s.write_string(fact_state_str(f.vector_paths))
    });
    s.write_field("form_xobjects", |s| {
        s.write_string(fact_state_str(f.form_xobjects))
    });
    s.write_field("annotations", |s| {
        s.write_string(fact_state_str(f.annotations))
    });
    s.write_field("link_annotations", |s| {
        s.write_string(fact_state_str(f.link_annotations))
    });
    s.write_field("widgets", |s| s.write_string(fact_state_str(f.widgets)));
    s.write_field("transparency", |s| {
        s.write_string(fact_state_str(f.transparency))
    });
    s.write_field("optional_content", |s| {
        s.write_string(fact_state_str(f.optional_content))
    });
    s.write_field("complex_resources", |s| {
        s.write_string(fact_state_str(f.complex_resources))
    });
    s.write_field("structure_association", |s| {
        s.write_string(fact_state_str(f.structure_association))
    });
    s.write_field("page_actions", |s| {
        s.write_string(fact_state_str(f.page_actions))
    });
    s.write_field("spot_colors", |s| {
        s.write_string(fact_state_str(f.spot_colors))
    });
    s.write_field("complex_color_spaces", |s| {
        s.write_string(fact_state_str(f.complex_color_spaces))
    });
    s.write_field("geometry_profile", |s| {
        s.write_string(f.geometry_profile.as_str())
    });
    s.write_field("rotation", |s| s.write_u32(f.rotation.as_u16() as u32));
}

fn write_document_redaction_facts(s: &mut CanonicalSerializer, f: &DocumentRedactionFacts) {
    s.write_object_start(18);
    s.write_field("schema_version", |s| s.write_string(&f.schema_version));
    s.write_field("total_pages", |s| s.write_u32(f.total_pages));
    s.write_field("encryption", |s| {
        s.write_string(fact_state_str(f.encryption))
    });
    s.write_field("signature_dictionary", |s| {
        s.write_string(fact_state_str(f.signature_dictionary))
    });
    s.write_field("signature_field", |s| {
        s.write_string(fact_state_str(f.signature_field))
    });
    s.write_field("byte_range", |s| {
        s.write_string(fact_state_str(f.byte_range))
    });
    s.write_field("acro_form", |s| s.write_string(fact_state_str(f.acro_form)));
    s.write_field("xfa", |s| s.write_string(fact_state_str(f.xfa)));
    s.write_field("annotations", |s| {
        s.write_string(fact_state_str(f.annotations))
    });
    s.write_field("optional_content", |s| {
        s.write_string(fact_state_str(f.optional_content))
    });
    s.write_field("embedded_files", |s| {
        s.write_string(fact_state_str(f.embedded_files))
    });
    s.write_field("javascript", |s| {
        s.write_string(fact_state_str(f.javascript))
    });
    s.write_field("incremental_revisions", |s| {
        s.write_string(fact_state_str(f.incremental_revisions))
    });
    s.write_field("structure_tree", |s| {
        s.write_string(fact_state_str(f.structure_tree))
    });
    s.write_field("outlines", |s| s.write_string(fact_state_str(f.outlines)));
    s.write_field("names_dictionary", |s| {
        s.write_string(fact_state_str(f.names_dictionary))
    });
    s.write_field("document_actions", |s| {
        s.write_string(fact_state_str(f.document_actions))
    });
    s.write_field("complex_document_resources", |s| {
        s.write_string(fact_state_str(f.complex_document_resources))
    });
}

fn write_redaction_feature_losses(s: &mut CanonicalSerializer, l: &RedactionFeatureLosses) {
    s.write_object_start(13);
    s.write_field("schema_version", |s| s.write_string(&l.schema_version));
    s.write_field("text_selection_lost", |s| {
        s.write_bool(l.text_selection_lost)
    });
    s.write_field("searchability_lost", |s| s.write_bool(l.searchability_lost));
    s.write_field("accessibility_tags_lost", |s| {
        s.write_bool(l.accessibility_tags_lost)
    });
    s.write_field("links_lost", |s| s.write_bool(l.links_lost));
    s.write_field("annotations_lost", |s| s.write_bool(l.annotations_lost));
    s.write_field("forms_lost", |s| s.write_bool(l.forms_lost));
    s.write_field("vector_quality_lost", |s| {
        s.write_bool(l.vector_quality_lost)
    });
    s.write_field("color_space_fidelity_risk", |s| {
        s.write_bool(l.color_space_fidelity_risk)
    });
    s.write_field("spot_colors_lost", |s| s.write_bool(l.spot_colors_lost));
    s.write_field("layers_lost", |s| s.write_bool(l.layers_lost));
    s.write_field("structure_association_lost", |s| {
        s.write_bool(l.structure_association_lost)
    });
    s.write_field("page_actions_lost", |s| s.write_bool(l.page_actions_lost));
}

fn write_secure_redaction_policy(s: &mut CanonicalSerializer, p: &SecureRedactionPolicy) {
    s.write_object_start(9);
    s.write_field("schema_version", |s| s.write_string(&p.schema_version));
    s.write_field("profile", |s| s.write_string("secure_rasterized_pages_v1"));
    s.write_field("strategy", |s| s.write_string("rasterized_pages"));
    s.write_field("raster_dpi", |s| s.write_u32(p.raster_dpi));
    s.write_field("appearance", |s| {
        s.write_object_start(3);
        s.write_field("kind", |s| s.write_string(&p.appearance.kind));
        s.write_field("color", |s| s.write_string(&p.appearance.color));
        s.write_field("opacity", |s| s.write_f64(p.appearance.opacity));
    });
    s.write_field("safety_margin_points", |s| {
        s.write_f64(p.safety_margin_points)
    });
    s.write_field("interactive_content_policy", |s| {
        s.write_string(match p.interactive_content_policy {
            InteractiveContentPolicy::Strip => "strip",
            InteractiveContentPolicy::Block => "block",
        })
    });
    s.write_field("signature_policy", |s| s.write_string("block"));
    s.write_field("encryption_policy", |s| s.write_string("block"));
}

fn write_redaction_acknowledgements(s: &mut CanonicalSerializer, a: &RedactionAcknowledgements) {
    s.write_object_start(8);
    s.write_field("schema_version", |s| s.write_string(&a.schema_version));
    s.write_field("rasterized_pages_accepted", |s| {
        s.write_bool(a.rasterized_pages_accepted)
    });
    s.write_field("text_search_loss_accepted", |s| {
        s.write_bool(a.text_search_loss_accepted)
    });
    s.write_field("accessibility_loss_accepted", |s| {
        s.write_bool(a.accessibility_loss_accepted)
    });
    s.write_field("annotation_loss_accepted", |s| {
        s.write_bool(a.annotation_loss_accepted)
    });
    s.write_field("form_loss_accepted", |s| s.write_bool(a.form_loss_accepted));
    s.write_field("signature_invalidation_accepted", |s| {
        s.write_bool(a.signature_invalidation_accepted)
    });
    s.write_field("encryption_change_accepted", |s| {
        s.write_bool(a.encryption_change_accepted)
    });
}

fn write_redaction_acknowledgement_requirements(
    s: &mut CanonicalSerializer,
    r: &RedactionAcknowledgementRequirements,
) {
    s.write_object_start(8);
    s.write_field("schema_version", |s| s.write_string(&r.schema_version));
    s.write_field("rasterized_pages_required", |s| {
        s.write_bool(r.rasterized_pages_required)
    });
    s.write_field("text_search_loss_required", |s| {
        s.write_bool(r.text_search_loss_required)
    });
    s.write_field("accessibility_loss_required", |s| {
        s.write_bool(r.accessibility_loss_required)
    });
    s.write_field("annotation_loss_required", |s| {
        s.write_bool(r.annotation_loss_required)
    });
    s.write_field("form_loss_required", |s| s.write_bool(r.form_loss_required));
    s.write_field("signature_invalidation_required", |s| {
        s.write_bool(r.signature_invalidation_required)
    });
    s.write_field("encryption_change_required", |s| {
        s.write_bool(r.encryption_change_required)
    });
}

fn write_page_redaction_plan(s: &mut CanonicalSerializer, p: &PageRedactionPlan) {
    s.write_object_start(11);
    s.write_field("schema_version", |s| s.write_string(&p.schema_version));
    s.write_field("page_index", |s| s.write_u32(p.page_index));
    s.write_field("strategy", |s| {
        s.write_string(match p.strategy {
            PageRedactionStrategy::RasterizePage => "rasterize_page",
            PageRedactionStrategy::ImportUnaffectedPage => "import_unaffected_page",
        })
    });
    s.write_field("geometry", |s| {
        write_canonical_page_geometry_fields(s, &p.geometry)
    });
    s.write_field("geometry_digest", |s| s.write_string(&p.geometry_digest));
    s.write_field("canonical_regions", |s| {
        s.write_list_start(p.canonical_regions.len());
        for r in &p.canonical_regions {
            write_canonical_region(s, r);
        }
    });
    s.write_field("facts", |s| write_page_redaction_facts(s, &p.facts));
    s.write_field("feature_losses", |s| {
        write_redaction_feature_losses(s, &p.feature_losses)
    });
    s.write_field("blockers", |s| {
        s.write_list_start(p.blockers.len());
        for b in &p.blockers {
            write_blocker(s, b);
        }
    });
    s.write_field("warnings", |s| {
        s.write_list_start(p.warnings.len());
        for w in &p.warnings {
            write_warning(s, w);
        }
    });
    s.write_field("executable", |s| s.write_bool(p.executable));
}

pub fn compute_plan_digest(
    input_digest: &str,
    content_generation: u64,
    policy: &SecureRedactionPolicy,
    acknowledgements: &RedactionAcknowledgements,
    ack_requirements: &RedactionAcknowledgementRequirements,
    facts: &DocumentRedactionFacts,
    page_plans: &[PageRedactionPlan],
    canonical_regions: &[CanonicalRedactionRegion],
    losses: &RedactionFeatureLosses,
    blockers: &[RedactionBlocker],
    warnings: &[RedactionWarning],
    executable: bool,
    acks_satisfied: bool,
    ready_for_execution: bool,
) -> String {
    let mut s = CanonicalSerializer::new();
    s.write_object_start(13);

    s.write_field("identity", |s| {
        s.write_object_start(4);
        s.write_field("schema_version", |s| {
            s.write_string(REDACTION_SCHEMA_VERSION)
        });
        s.write_field("plan_version", |s| {
            s.write_string(SECURE_REDACTION_PLAN_VERSION)
        });
        s.write_field("input_digest", |s| s.write_string(input_digest));
        s.write_field("content_generation", |s| s.write_u64(content_generation));
    });
    s.write_field("policy", |s| write_secure_redaction_policy(s, policy));
    s.write_field("merge_profile", |s| {
        s.write_object_start(4);
        s.write_field("adjacency_tolerance_points", |s| {
            s.write_f64(ADJACENCY_TOLERANCE_POINTS)
        });
        s.write_field("merge_edge_touching", |s| s.write_bool(MERGE_EDGE_TOUCHING));
        s.write_field("merge_corner_touching", |s| {
            s.write_bool(MERGE_CORNER_TOUCHING)
        });
        s.write_field("geometry_comparison_epsilon", |s| {
            s.write_f64(GEOMETRY_COMPARISON_EPSILON)
        });
    });
    s.write_field("acknowledgements", |s| {
        write_redaction_acknowledgements(s, acknowledgements)
    });
    s.write_field("acknowledgement_requirements", |s| {
        write_redaction_acknowledgement_requirements(s, ack_requirements)
    });
    s.write_field("facts", |s| write_document_redaction_facts(s, facts));
    s.write_field("losses", |s| write_redaction_feature_losses(s, losses));
    s.write_field("readiness", |s| {
        s.write_object_start(3);
        s.write_field("executable", |s| s.write_bool(executable));
        s.write_field("acknowledgements_satisfied", |s| {
            s.write_bool(acks_satisfied)
        });
        s.write_field("ready_for_execution", |s| s.write_bool(ready_for_execution));
    });
    s.write_field("page_plans", |s| {
        s.write_list_start(page_plans.len());
        for p in page_plans {
            write_page_redaction_plan(s, p);
        }
    });
    s.write_field("canonical_regions", |s| {
        s.write_list_start(canonical_regions.len());
        for r in canonical_regions {
            write_canonical_region(s, r);
        }
    });
    s.write_field("blockers", |s| {
        s.write_list_start(blockers.len());
        for b in blockers {
            write_blocker(s, b);
        }
    });
    s.write_field("warnings", |s| {
        s.write_list_start(warnings.len());
        for w in warnings {
            write_warning(s, w);
        }
    });

    s.finalize_digest()
}

pub fn build_secure_redaction_plan(
    input: &RedactionPlannerInput,
) -> Result<SecureRedactionPlan, SessionJobOperationError> {
    validate_redaction_planner_input(input)?;

    let mut doc_blockers = evaluate_document_blockers(
        &input.document_facts,
        &input.policy,
        &input.acknowledgements,
        None,
    );
    let mut warnings: Vec<RedactionWarning> = Vec::new();

    if input.region_inputs.len() > MAX_REGIONS_PER_INPUT {
        push_aggregated_blocker(
            &mut doc_blockers,
            RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_RESOURCE_LIMIT_EXCEEDED".to_string(),
                message: format!(
                    "Input region count ({}) exceeds maximum limit of {}",
                    input.region_inputs.len(),
                    MAX_REGIONS_PER_INPUT
                ),
                details: None,
            },
        );
    }

    if input.document_facts.total_pages as usize > MAX_PAGE_PLANS {
        push_aggregated_blocker(
            &mut doc_blockers,
            RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_RESOURCE_LIMIT_EXCEEDED".to_string(),
                message: format!(
                    "Total page count ({}) exceeds maximum page plan limit of {}",
                    input.document_facts.total_pages, MAX_PAGE_PLANS
                ),
                details: None,
            },
        );
    }

    let geom_by_page: HashMap<u32, &CanonicalPageGeometry> = input
        .page_geometries
        .iter()
        .map(|g| (g.page_index, g))
        .collect();

    let facts_by_page: HashMap<u32, &PageRedactionFacts> =
        input.page_facts.iter().map(|f| (f.page_index, f)).collect();

    for page_idx in 0..input.document_facts.total_pages {
        let g = match geom_by_page.get(&page_idx) {
            Some(g) => g,
            None => {
                return Err(make_redaction_error(
                    SecureRedactionErrorCode::RedactionPageNotFound,
                    format!("Missing CanonicalPageGeometry for page {}", page_idx),
                ));
            }
        };
        let f = match facts_by_page.get(&page_idx) {
            Some(f) => f,
            None => {
                return Err(make_redaction_error(
                    SecureRedactionErrorCode::RedactionPageNotFound,
                    format!("Missing PageRedactionFacts for page {}", page_idx),
                ));
            }
        };
        if g.rotation != f.rotation {
            return Err(make_redaction_error(
                SecureRedactionErrorCode::RedactionPageGeometryMismatch,
                format!(
                    "Geometry rotation ({:?}) mismatch with page facts rotation ({:?}) on page {}",
                    g.rotation, f.rotation, page_idx
                ),
            ));
        }
    }

    let mut regions_by_page: HashMap<u32, Vec<&RedactionRegionInput>> = HashMap::new();
    let mut sorted_inputs = input.region_inputs.clone();
    sorted_inputs.sort_by(|a, b| {
        a.page_index
            .cmp(&b.page_index)
            .then_with(|| {
                a.normalized_top_left_rect
                    .y
                    .partial_cmp(&b.normalized_top_left_rect.y)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                a.normalized_top_left_rect
                    .x
                    .partial_cmp(&b.normalized_top_left_rect.x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.id.cmp(&b.id))
    });

    for reg in &sorted_inputs {
        regions_by_page.entry(reg.page_index).or_default().push(reg);
    }

    let mut page_plans = Vec::new();
    let total_pages = input.document_facts.total_pages;

    for page_index in 0..total_pages {
        let geom = geom_by_page.get(&page_index).unwrap();
        let page_facts = facts_by_page.get(&page_index).unwrap();

        let geom_digest = compute_geometry_digest(geom);
        let raw_regions = regions_by_page.get(&page_index);

        let mut canonical_unmerged = Vec::new();
        let mut page_blockers = evaluate_page_redaction_blockers(page_facts);
        let mut page_warnings = Vec::new();

        if let Some(r_list) = raw_regions {
            for raw in r_list {
                let pdf_point_rect =
                    normalized_top_left_to_pdf_point_rect(&raw.normalized_top_left_rect, geom)?;

                let (base_rect, applied_rect, applied_margins, boundary_adjusted) =
                    apply_safety_margin_and_clamp(
                        &pdf_point_rect,
                        input.policy.safety_margin_points,
                        &geom.effective_box,
                    );

                if boundary_adjusted {
                    push_aggregated_warning(
                        &mut page_warnings,
                        RedactionWarning {
                            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                            code: "REDACTION_REGION_SAFETY_MARGIN_CLAMPED".to_string(),
                            message: format!(
                                "Region '{}' on page {} safety margin extended outside page effective box and was clamped",
                                raw.id, page_index
                            ),
                            details: Some(format!(
                                "Region {} (page {}): left={:.2}, bottom={:.2}, width={:.2}, height={:.2}",
                                raw.id, page_index, applied_rect.left, applied_rect.bottom, applied_rect.width, applied_rect.height
                            )),
                        },
                    );
                }

                if applied_rect.width <= 0.0 || applied_rect.height <= 0.0 {
                    push_aggregated_blocker(
                        &mut page_blockers,
                        RedactionBlocker {
                            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                            code: "REDACTION_REGION_OUT_OF_BOUNDS".to_string(),
                            message: format!(
                                "Region '{}' on page {} is completely outside page effective box bounds after transformation/clamping",
                                raw.id, page_index
                            ),
                            details: None,
                        },
                    );
                } else {
                    canonical_unmerged.push(CanonicalRedactionRegion {
                        region_id: raw.id.clone(),
                        source_region_ids: vec![raw.id.clone()],
                        page_index,
                        original_normalized_rect: raw.normalized_top_left_rect.clone(),
                        base_pdf_point_rect: base_rect,
                        requested_safety_margin_points: input.policy.safety_margin_points,
                        applied_pdf_point_rect: applied_rect,
                        applied_margins,
                        boundary_adjusted,
                        device_pixel_rect: None,
                    });
                }
            }
        }

        let merge_outcome = merge_canonical_regions_on_page(canonical_unmerged, page_index)?;
        let merged_canonical_regions = merge_outcome.merged_regions;

        if merge_outcome.resource_limit_exceeded {
            push_aggregated_blocker(
                &mut page_blockers,
                RedactionBlocker {
                    schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                    code: "REDACTION_RESOURCE_LIMIT_EXCEEDED".to_string(),
                    message: format!(
                        "Page {} merged canonical region source_region_ids count would exceed limit of {}",
                        page_index,
                        MAX_SOURCE_REGION_IDS
                    ),
                    details: None,
                },
            );
        }

        if merged_canonical_regions.len() > MAX_CANONICAL_REGIONS_PER_PAGE {
            push_aggregated_blocker(
                &mut page_blockers,
                RedactionBlocker {
                    schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                    code: "REDACTION_RESOURCE_LIMIT_EXCEEDED".to_string(),
                    message: format!(
                        "Page {} merged canonical regions count ({}) exceeds maximum limit of {}",
                        page_index,
                        merged_canonical_regions.len(),
                        MAX_CANONICAL_REGIONS_PER_PAGE
                    ),
                    details: None,
                },
            );
        }

        let is_affected = !merged_canonical_regions.is_empty();
        let strategy = if is_affected {
            PageRedactionStrategy::RasterizePage
        } else {
            PageRedactionStrategy::ImportUnaffectedPage
        };

        // Keep planning and execution capability-identical. The worker repeats
        // this check before mutation as defense in depth, but must never invent
        // a different geometry rejection after an executable plan is issued.
        if is_affected {
            if let Some(blocker) = secure_redaction_rebuild_geometry_blocker(geom) {
                push_aggregated_blocker(&mut page_blockers, blocker);
            }
        }

        // Section 10.1 & 10.2: Feature Loss Calculations strictly bound to actual facts
        let feature_losses = if is_affected {
            RedactionFeatureLosses {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                text_selection_lost: page_facts.text.is_present(),
                searchability_lost: page_facts.text.is_present(),
                accessibility_tags_lost: page_facts.structure_association.is_present(),
                links_lost: page_facts.link_annotations.is_present(),
                annotations_lost: page_facts.annotations.is_present(),
                forms_lost: page_facts.widgets.is_present(),
                vector_quality_lost: page_facts.vector_paths.is_present()
                    || page_facts.form_xobjects.is_present(),
                color_space_fidelity_risk: page_facts.complex_resources.is_present()
                    || page_facts.transparency.is_present()
                    || page_facts.complex_color_spaces.is_present(),
                spot_colors_lost: page_facts.spot_colors.is_present(),
                layers_lost: page_facts.optional_content.is_present(),
                structure_association_lost: page_facts.structure_association.is_present(),
                page_actions_lost: page_facts.page_actions.is_present(),
            }
        } else {
            RedactionFeatureLosses {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                text_selection_lost: false,
                searchability_lost: false,
                accessibility_tags_lost: false,
                links_lost: false,
                annotations_lost: false,
                forms_lost: false,
                vector_quality_lost: false,
                color_space_fidelity_risk: false,
                spot_colors_lost: false,
                layers_lost: false,
                structure_association_lost: false,
                page_actions_lost: false,
            }
        };

        let page_executable = page_blockers.is_empty();

        let page_plan = PageRedactionPlan {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            page_index,
            strategy,
            geometry: (*geom).clone(),
            geometry_digest: geom_digest,
            canonical_regions: merged_canonical_regions,
            facts: (*page_facts).clone(),
            feature_losses,
            blockers: page_blockers,
            warnings: page_warnings,
            executable: page_executable,
        };

        page_plans.push(page_plan);
    }

    let mut total_estimated_pixels: u64 = 0;
    for page_plan in &page_plans {
        if page_plan.strategy == PageRedactionStrategy::RasterizePage {
            let width_px = (page_plan.geometry.effective_box.width / 72.0
                * input.policy.raster_dpi as f64)
                .ceil() as u64;
            let height_px = (page_plan.geometry.effective_box.height / 72.0
                * input.policy.raster_dpi as f64)
                .ceil() as u64;
            let page_pixels = match width_px.checked_mul(height_px) {
                Some(px) => px,
                None => {
                    push_aggregated_blocker(
                        &mut doc_blockers,
                        RedactionBlocker {
                            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                            code: "REDACTION_RESOURCE_LIMIT_EXCEEDED".to_string(),
                            message: format!(
                                "Pixel count arithmetic overflow on page {}",
                                page_plan.page_index
                            ),
                            details: None,
                        },
                    );
                    u64::MAX
                }
            };
            if page_pixels > MAX_PIXELS_PER_PAGE {
                push_aggregated_blocker(
                    &mut doc_blockers,
                    RedactionBlocker {
                        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                        code: "REDACTION_RESOURCE_LIMIT_EXCEEDED".to_string(),
                        message: format!(
                            "Page {} estimated pixel count ({}) exceeds limit of {}",
                            page_plan.page_index, page_pixels, MAX_PIXELS_PER_PAGE
                        ),
                        details: None,
                    },
                );
            }
            total_estimated_pixels = total_estimated_pixels.saturating_add(page_pixels);
        }
    }

    if total_estimated_pixels > MAX_TOTAL_ESTIMATED_PIXELS {
        push_aggregated_blocker(
            &mut doc_blockers,
            RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_RESOURCE_LIMIT_EXCEEDED".to_string(),
                message: format!(
                    "Total estimated pixels ({}) exceeds limit of {}",
                    total_estimated_pixels, MAX_TOTAL_ESTIMATED_PIXELS
                ),
                details: None,
            },
        );
    }

    let mut doc_text_lost = false;
    let mut doc_search_lost = false;
    let mut doc_a11y_lost = false;
    let mut doc_links_lost = false;
    let mut doc_annot_lost = false;
    let mut doc_forms_lost = false;
    let mut doc_vector_rasterized = false;
    let mut doc_color_risk = false;
    let mut doc_spot_lost = false;
    let mut doc_layers_lost = false;
    let mut doc_struct_lost = false;
    let mut doc_actions_lost = false;

    for pp in &page_plans {
        if pp.feature_losses.text_selection_lost {
            doc_text_lost = true;
        }
        if pp.feature_losses.searchability_lost {
            doc_search_lost = true;
        }
        if pp.feature_losses.accessibility_tags_lost {
            doc_a11y_lost = true;
        }
        if pp.feature_losses.links_lost {
            doc_links_lost = true;
        }
        if pp.feature_losses.annotations_lost {
            doc_annot_lost = true;
        }
        if pp.feature_losses.forms_lost {
            doc_forms_lost = true;
        }
        if pp.feature_losses.vector_quality_lost {
            doc_vector_rasterized = true;
        }
        if pp.feature_losses.color_space_fidelity_risk {
            doc_color_risk = true;
        }
        if pp.feature_losses.spot_colors_lost {
            doc_spot_lost = true;
        }
        if pp.feature_losses.layers_lost {
            doc_layers_lost = true;
        }
        if pp.feature_losses.structure_association_lost {
            doc_struct_lost = true;
        }
        if pp.feature_losses.page_actions_lost {
            doc_actions_lost = true;
        }
    }

    let doc_losses = RedactionFeatureLosses {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        text_selection_lost: doc_text_lost,
        searchability_lost: doc_search_lost,
        accessibility_tags_lost: doc_a11y_lost,
        links_lost: doc_links_lost,
        annotations_lost: doc_annot_lost,
        forms_lost: doc_forms_lost,
        vector_quality_lost: doc_vector_rasterized,
        color_space_fidelity_risk: doc_color_risk,
        spot_colors_lost: doc_spot_lost,
        layers_lost: doc_layers_lost,
        structure_association_lost: doc_struct_lost,
        page_actions_lost: doc_actions_lost,
    };

    let ack_requirements = derive_redaction_acknowledgement_requirements(
        &input.document_facts,
        &doc_losses,
        &page_plans,
    );

    let mut all_canonical_regions = Vec::new();
    for pp in &page_plans {
        all_canonical_regions.extend(pp.canonical_regions.clone());
    }
    all_canonical_regions.sort_by(compare_canonical_redaction_regions);

    let doc_blockers = evaluate_document_blockers(
        &input.document_facts,
        &input.policy,
        &input.acknowledgements,
        Some(&page_plans),
    );
    let mut all_blockers = doc_blockers;
    for pp in &page_plans {
        for b in &pp.blockers {
            push_aggregated_blocker(&mut all_blockers, b.clone());
        }
        for w in &pp.warnings {
            push_aggregated_warning(&mut warnings, w.clone());
        }
    }

    let executable = all_blockers.is_empty();
    let acks_satisfied =
        evaluate_acknowledgements_satisfied(&ack_requirements, &input.acknowledgements);
    let ready_for_execution = executable && acks_satisfied;

    let plan_digest = compute_plan_digest(
        &input.input_digest,
        input.content_generation,
        &input.policy,
        &input.acknowledgements,
        &ack_requirements,
        &input.document_facts,
        &page_plans,
        &all_canonical_regions,
        &doc_losses,
        &all_blockers,
        &warnings,
        executable,
        acks_satisfied,
        ready_for_execution,
    );

    let plan = SecureRedactionPlan {
        schema_version: REDACTION_SCHEMA_VERSION.to_string(),
        plan_version: SECURE_REDACTION_PLAN_VERSION.to_string(),
        input_digest: input.input_digest.clone(),
        content_generation: input.content_generation,
        plan_digest,
        policy: input.policy.clone(),
        acknowledgements: input.acknowledgements.clone(),
        acknowledgement_requirements: ack_requirements,
        page_plans,
        canonical_regions: all_canonical_regions,
        facts: input.document_facts.clone(),
        losses: doc_losses,
        blockers: all_blockers,
        warnings,
        executable,
        acknowledgements_satisfied: acks_satisfied,
        ready_for_execution,
    };

    let serialized_plan = serde_json::to_vec(&plan).map_err(|e| {
        make_redaction_error(
            SecureRedactionErrorCode::RedactionInvalidPolicy,
            format!("Failed to serialize secure redaction plan: {}", e),
        )
    })?;
    let serialized_plan_bytes = serialized_plan.len();
    if serialized_plan_bytes > MAX_REPORT_SIZE_BYTES {
        return Err(make_redaction_error(
            SecureRedactionErrorCode::RedactionResourceLimitExceeded,
            format!(
                "Serialized plan size ({} bytes) exceeds report size limit of {} bytes",
                serialized_plan_bytes, MAX_REPORT_SIZE_BYTES
            ),
        ));
    }

    validate_secure_redaction_plan(&plan)?;

    Ok(plan)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_sha256() -> String {
        "a".repeat(64)
    }

    fn required_verification_checks(
        status: VerificationCheckStatus,
    ) -> Vec<RedactionVerificationCheck> {
        REQUIRED_SECURE_REDACTION_VERIFICATION_CHECKS
            .iter()
            .map(|code| RedactionVerificationCheck {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: (*code).to_string(),
                status,
                message: "check".to_string(),
                details: None,
            })
            .collect()
    }

    fn sample_transform_report() -> SecureRedactionTransformReport {
        SecureRedactionTransformReport {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            job_id: "job_1".to_string(),
            session_id: "sess_1".to_string(),
            input_digest: sample_sha256(),
            plan_digest: sample_sha256(),
            output_relative_name: "secure-redaction-transform.pdf".to_string(),
            output_digest: sample_sha256(),
            output_size_bytes: 1,
            page_count: 0,
            affected_pages: vec![],
            unaffected_pages: vec![],
            raster_dpi: MIN_RASTER_DPI,
            applied_canonical_regions: vec![],
            affected_page_raster_geometry: vec![],
            page_feature_losses: vec![],
            background_policy: "opaque_white_v1".to_string(),
            warnings: vec![],
            transform_evidence: SecureRedactionTransformEvidence {
                new_document_constructed: true,
                affected_pages_rasterized: true,
                affected_page_original_objects_not_imported: true,
                unaffected_pages_imported: true,
                full_rewrite_used: true,
                incremental_save_not_used: true,
                opaque_white_background_policy: true,
                lossless_raster_path: true,
                output_digest_calculated: true,
            },
            status: "transformed_unverified".to_string(),
        }
    }

    fn sample_populated_transform_report() -> SecureRedactionTransformReport {
        let page_plan = sample_valid_page_plan();
        let mut report = sample_transform_report();
        report.page_count = 1;
        report.affected_pages = vec![0];
        report.applied_canonical_regions = page_plan.canonical_regions;
        report.affected_page_raster_geometry = vec![TransformRasterPageEvidence {
            page_index: 0,
            width: 100,
            height: 100,
            dpi: MIN_RASTER_DPI,
        }];
        report.page_feature_losses = vec![TransformPageFeatureLoss {
            page_index: 0,
            feature_losses: page_plan.feature_losses,
        }];
        report
    }

    fn sample_valid_input(id: &str, x: f64, y: f64, w: f64, h: f64) -> RedactionRegionInput {
        RedactionRegionInput {
            id: id.to_string(),
            page_index: 0,
            normalized_top_left_rect: NormalizedTopLeftRect {
                x,
                y,
                width: w,
                height: h,
            },
            source: "manual_selection".to_string(),
        }
    }

    fn sample_valid_policy() -> SecureRedactionPolicy {
        SecureRedactionPolicy {
            schema_version: "1.0.0".to_string(),
            profile: RedactionProfile::SecureRasterizedPagesV1,
            strategy: RedactionStrategy::RasterizedPages,
            raster_dpi: 300,
            appearance: RedactionAppearancePolicy {
                kind: "solid".to_string(),
                color: "black".to_string(),
                opacity: 1.0,
            },
            safety_margin_points: 2.0,
            interactive_content_policy: InteractiveContentPolicy::Block,
            signature_policy: SignaturePolicy::Block,
            encryption_policy: EncryptionPolicy::Block,
        }
    }

    fn sample_valid_acknowledgements() -> RedactionAcknowledgements {
        RedactionAcknowledgements {
            schema_version: "1.0.0".to_string(),
            rasterized_pages_accepted: true,
            text_search_loss_accepted: true,
            accessibility_loss_accepted: true,
            annotation_loss_accepted: true,
            form_loss_accepted: true,
            signature_invalidation_accepted: true,
            encryption_change_accepted: true,
        }
    }

    fn sample_valid_geometry() -> CanonicalPageGeometry {
        CanonicalPageGeometry {
            page_index: 0,
            media_box: PdfPointRect {
                left: 0.0,
                bottom: 0.0,
                width: 612.0,
                height: 792.0,
            },
            crop_box: None,
            trim_box: None,
            bleed_box: None,
            art_box: None,
            effective_box: PdfPointRect {
                left: 0.0,
                bottom: 0.0,
                width: 612.0,
                height: 792.0,
            },
            rotation: PageRotation::Deg0,
        }
    }

    fn sample_valid_page_plan() -> PageRedactionPlan {
        let geom = sample_valid_geometry();
        let geom_digest = compute_geometry_digest(&geom);
        PageRedactionPlan {
            schema_version: "1.0.0".to_string(),
            page_index: 0,
            strategy: PageRedactionStrategy::RasterizePage,
            geometry: geom,
            geometry_digest: geom_digest,
            canonical_regions: vec![CanonicalRedactionRegion {
                region_id: "canon_r1".to_string(),
                source_region_ids: vec!["s1".to_string()],
                page_index: 0,
                original_normalized_rect: NormalizedTopLeftRect {
                    x: 0.1,
                    y: 0.1,
                    width: 0.3,
                    height: 0.2,
                },
                base_pdf_point_rect: PdfPointRect {
                    left: 61.2,
                    bottom: 554.4,
                    width: 183.6,
                    height: 158.4,
                },
                requested_safety_margin_points: 2.0,
                applied_pdf_point_rect: PdfPointRect {
                    left: 59.2,
                    bottom: 552.4,
                    width: 187.6,
                    height: 162.4,
                },
                applied_margins: AppliedMargins {
                    left: 2.0,
                    right: 2.0,
                    top: 2.0,
                    bottom: 2.0,
                },
                boundary_adjusted: false,
                device_pixel_rect: None,
            }],
            facts: PageRedactionFacts {
                schema_version: "1.0.0".to_string(),
                page_index: 0,
                text: InspectionFactState::Present,
                images: InspectionFactState::Absent,
                vector_paths: InspectionFactState::Absent,
                form_xobjects: InspectionFactState::Absent,
                annotations: InspectionFactState::Absent,
                link_annotations: InspectionFactState::Absent,
                widgets: InspectionFactState::Absent,
                transparency: InspectionFactState::Absent,
                optional_content: InspectionFactState::Absent,
                complex_resources: InspectionFactState::Absent,
                structure_association: InspectionFactState::Absent,
                page_actions: InspectionFactState::Absent,
                spot_colors: InspectionFactState::Absent,
                complex_color_spaces: InspectionFactState::Absent,
                geometry_profile: GeometryProfile::StandardEffectiveBoxV1,
                rotation: PageRotation::Deg0,
            },
            feature_losses: RedactionFeatureLosses {
                schema_version: "1.0.0".to_string(),
                text_selection_lost: true,
                searchability_lost: true,
                accessibility_tags_lost: false,
                links_lost: false,
                annotations_lost: false,
                forms_lost: false,
                vector_quality_lost: false,
                color_space_fidelity_risk: false,
                spot_colors_lost: false,
                layers_lost: false,
                structure_association_lost: false,
                page_actions_lost: false,
            },
            blockers: vec![],
            warnings: vec![],
            executable: true,
        }
    }

    /// This is intentionally constructed as a typed DTO rather than obtained
    /// by deserializing the fixture: it makes the fixture an exact assertion of
    /// the public serde wire contract, including null option fields.
    fn canonical_wire_fixture_plan() -> SecureRedactionPlan {
        let mut page_plan = sample_valid_page_plan();
        page_plan.geometry = CanonicalPageGeometry {
            page_index: 0,
            media_box: PdfPointRect {
                left: 0.0,
                bottom: 0.0,
                width: 612.0,
                height: 792.0,
            },
            crop_box: None,
            trim_box: None,
            bleed_box: None,
            art_box: None,
            effective_box: PdfPointRect {
                left: 0.0,
                bottom: 0.0,
                width: 612.0,
                height: 792.0,
            },
            rotation: PageRotation::Deg0,
        };
        page_plan.geometry_digest = compute_geometry_digest(&page_plan.geometry);
        page_plan.canonical_regions[0].region_id = "canon_r1".to_string();
        page_plan.canonical_regions[0].source_region_ids = vec!["s1".to_string()];
        page_plan.canonical_regions[0].original_normalized_rect = NormalizedTopLeftRect {
            x: 0.1,
            y: 0.1,
            width: 0.3,
            height: 0.2,
        };
        page_plan.canonical_regions[0].base_pdf_point_rect = PdfPointRect {
            left: 61.2,
            bottom: 554.4,
            width: 183.6,
            height: 158.4,
        };
        page_plan.canonical_regions[0].requested_safety_margin_points = 2.0;
        page_plan.canonical_regions[0].applied_pdf_point_rect = PdfPointRect {
            left: 59.2,
            bottom: 552.4,
            width: 187.6,
            height: 162.4,
        };
        page_plan.canonical_regions[0].applied_margins = AppliedMargins {
            left: 2.0,
            right: 2.0,
            top: 2.0,
            bottom: 2.0,
        };
        page_plan.canonical_regions[0].boundary_adjusted = false;

        let canonical_regions = page_plan.canonical_regions.clone();
        let policy = sample_valid_policy();
        let acks = sample_valid_acknowledgements();
        let ack_reqs = RedactionAcknowledgementRequirements {
            schema_version: "1.0.0".to_string(),
            rasterized_pages_required: true,
            text_search_loss_required: true,
            accessibility_loss_required: false,
            annotation_loss_required: false,
            form_loss_required: false,
            signature_invalidation_required: false,
            encryption_change_required: false,
        };
        let doc_facts = DocumentRedactionFacts {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            total_pages: 1,
            encryption: InspectionFactState::Absent,
            signature_dictionary: InspectionFactState::Absent,
            signature_field: InspectionFactState::Absent,
            byte_range: InspectionFactState::Absent,
            acro_form: InspectionFactState::Absent,
            xfa: InspectionFactState::Absent,
            annotations: InspectionFactState::Absent,
            optional_content: InspectionFactState::Absent,
            embedded_files: InspectionFactState::Absent,
            javascript: InspectionFactState::Absent,
            incremental_revisions: InspectionFactState::Absent,
            structure_tree: InspectionFactState::Absent,
            outlines: InspectionFactState::Absent,
            names_dictionary: InspectionFactState::Absent,
            document_actions: InspectionFactState::Absent,
            complex_document_resources: InspectionFactState::Absent,
        };
        let doc_losses = RedactionFeatureLosses {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            text_selection_lost: true,
            searchability_lost: true,
            accessibility_tags_lost: false,
            links_lost: false,
            annotations_lost: false,
            forms_lost: false,
            vector_quality_lost: false,
            color_space_fidelity_risk: false,
            spot_colors_lost: false,
            layers_lost: false,
            structure_association_lost: false,
            page_actions_lost: false,
        };

        let plan_digest = compute_plan_digest(
            &"a0".repeat(32),
            0,
            &policy,
            &acks,
            &ack_reqs,
            &doc_facts,
            &[page_plan.clone()],
            &canonical_regions,
            &doc_losses,
            &[],
            &[],
            true,
            true,
            true,
        );

        SecureRedactionPlan {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            plan_version: SECURE_REDACTION_PLAN_VERSION.to_string(),
            input_digest: "a0".repeat(32),
            content_generation: 0,
            plan_digest,
            policy,
            acknowledgements: acks,
            acknowledgement_requirements: ack_reqs,
            page_plans: vec![page_plan],
            canonical_regions,
            facts: doc_facts,
            losses: doc_losses,
            blockers: vec![],
            warnings: vec![],
            executable: true,
            acknowledgements_satisfied: true,
            ready_for_execution: true,
        }
    }

    fn sample_valid_plan() -> SecureRedactionPlan {
        canonical_wire_fixture_plan()
    }

    #[test]
    fn test_policy_validation_and_appearance_opacity() {
        let policy = sample_valid_policy();
        assert!(validate_secure_redaction_policy(&policy).is_ok());

        let mut invalid_policy = policy.clone();
        invalid_policy.appearance.opacity = 0.999;
        assert!(validate_secure_redaction_policy(&invalid_policy).is_err());
    }

    #[test]
    fn test_acknowledgements_validation() {
        let ack = sample_valid_acknowledgements();
        assert!(validate_redaction_acknowledgements(&ack).is_ok());

        let mut invalid_ack = ack.clone();
        invalid_ack.schema_version = "2.0.0".to_string();
        assert!(validate_redaction_acknowledgements(&invalid_ack).is_err());
    }

    #[test]
    fn test_exact_bounds_validation_without_epsilon() {
        let input_exact = sample_valid_input("r1", 0.0, 0.0, 1.0, 1.0);
        assert!(validate_redaction_region_input(&input_exact).is_ok());

        let input_tiny_positive = sample_valid_input("r_tiny", 0.0, 0.0, 1e-8, 1e-8);
        assert!(validate_redaction_region_input(&input_tiny_positive).is_ok());

        let input_overflow = sample_valid_input("r2", -0.0000005, 0.0, 0.5, 0.5);
        assert!(validate_redaction_region_input(&input_overflow).is_err());

        let input_overflow_right = sample_valid_input("r3", 0.5, 0.0, 0.5000005, 0.5);
        assert!(validate_redaction_region_input(&input_overflow_right).is_err());
    }

    #[test]
    fn test_duplicate_blocker_code_policy_rejection() {
        let b1 = RedactionBlocker {
            schema_version: "1.0.0".to_string(),
            code: "BLOCKER_1".to_string(),
            message: "msg 1".to_string(),
            details: None,
        };
        let b2 = RedactionBlocker {
            schema_version: "1.0.0".to_string(),
            code: "BLOCKER_1".to_string(),
            message: "msg 2".to_string(),
            details: None,
        };
        let res = validate_unique_blockers(&[b1, b2]);
        assert!(res.is_err());
        if let crate::document_session::SessionJobOperationError::Failed(f) = res.unwrap_err() {
            assert_eq!(f.code, "REDACTION_INVALID_POLICY");
        } else {
            panic!("Expected Failed error");
        }
    }

    #[test]
    fn test_valid_secure_redaction_plan_happy_path() {
        let digest = sample_sha256();
        let page_plan = sample_valid_page_plan();
        let top_regions = page_plan.canonical_regions.clone();
        let policy = sample_valid_policy();
        let acks = sample_valid_acknowledgements();
        let ack_reqs = RedactionAcknowledgementRequirements {
            schema_version: "1.0.0".to_string(),
            rasterized_pages_required: true,
            text_search_loss_required: true,
            accessibility_loss_required: false,
            annotation_loss_required: false,
            form_loss_required: false,
            signature_invalidation_required: false,
            encryption_change_required: false,
        };
        let doc_facts = DocumentRedactionFacts {
            schema_version: "1.0.0".to_string(),
            total_pages: 1,
            encryption: InspectionFactState::Absent,
            signature_dictionary: InspectionFactState::Absent,
            signature_field: InspectionFactState::Absent,
            byte_range: InspectionFactState::Absent,
            acro_form: InspectionFactState::Absent,
            xfa: InspectionFactState::Absent,
            annotations: InspectionFactState::Absent,
            optional_content: InspectionFactState::Absent,
            embedded_files: InspectionFactState::Absent,
            javascript: InspectionFactState::Absent,
            incremental_revisions: InspectionFactState::Absent,
            structure_tree: InspectionFactState::Absent,
            outlines: InspectionFactState::Absent,
            names_dictionary: InspectionFactState::Absent,
            document_actions: InspectionFactState::Absent,
            complex_document_resources: InspectionFactState::Absent,
        };
        let losses = RedactionFeatureLosses {
            schema_version: "1.0.0".to_string(),
            text_selection_lost: true,
            searchability_lost: true,
            accessibility_tags_lost: false,
            links_lost: false,
            annotations_lost: false,
            forms_lost: false,
            vector_quality_lost: false,
            color_space_fidelity_risk: false,
            spot_colors_lost: false,
            layers_lost: false,
            structure_association_lost: false,
            page_actions_lost: false,
        };

        let plan_digest = compute_plan_digest(
            &digest,
            0,
            &policy,
            &acks,
            &ack_reqs,
            &doc_facts,
            &[page_plan.clone()],
            &top_regions,
            &losses,
            &[],
            &[],
            true,
            true,
            true,
        );

        let plan = SecureRedactionPlan {
            schema_version: "1.0.0".to_string(),
            plan_version: SECURE_REDACTION_PLAN_VERSION.to_string(),
            input_digest: digest.clone(),
            content_generation: 0,
            plan_digest,
            policy,
            acknowledgements: acks,
            acknowledgement_requirements: ack_reqs,
            page_plans: vec![page_plan],
            canonical_regions: top_regions,
            facts: doc_facts,
            losses,
            blockers: vec![],
            warnings: vec![],
            executable: true,
            acknowledgements_satisfied: true,
            ready_for_execution: true,
        };

        assert!(validate_secure_redaction_plan(&plan).is_ok());
    }

    #[test]
    fn test_plan_validation_nested_invariants() {
        let digest = sample_sha256();
        let page_plan = sample_valid_page_plan();
        let top_regions = page_plan.canonical_regions.clone();
        let valid_plan = SecureRedactionPlan {
            schema_version: "1.0.0".to_string(),
            plan_version: SECURE_REDACTION_PLAN_VERSION.to_string(),
            input_digest: digest.clone(),
            content_generation: 0,
            plan_digest: "c0".repeat(32),
            policy: sample_valid_policy(),
            acknowledgements: sample_valid_acknowledgements(),
            acknowledgement_requirements: RedactionAcknowledgementRequirements {
                schema_version: "1.0.0".to_string(),
                rasterized_pages_required: true,
                text_search_loss_required: false,
                accessibility_loss_required: false,
                annotation_loss_required: false,
                form_loss_required: false,
                signature_invalidation_required: false,
                encryption_change_required: false,
            },
            page_plans: vec![page_plan],
            canonical_regions: top_regions,
            facts: DocumentRedactionFacts {
                schema_version: "1.0.0".to_string(),
                total_pages: 1,
                encryption: InspectionFactState::Absent,
                signature_dictionary: InspectionFactState::Absent,
                signature_field: InspectionFactState::Absent,
                byte_range: InspectionFactState::Absent,
                acro_form: InspectionFactState::Absent,
                xfa: InspectionFactState::Absent,
                annotations: InspectionFactState::Absent,
                optional_content: InspectionFactState::Absent,
                embedded_files: InspectionFactState::Absent,
                javascript: InspectionFactState::Absent,
                incremental_revisions: InspectionFactState::Absent,
                structure_tree: InspectionFactState::Absent,
                outlines: InspectionFactState::Absent,
                names_dictionary: InspectionFactState::Absent,
                document_actions: InspectionFactState::Absent,
                complex_document_resources: InspectionFactState::Absent,
            },
            losses: RedactionFeatureLosses {
                schema_version: "1.0.0".to_string(),
                text_selection_lost: true,
                searchability_lost: true,
                accessibility_tags_lost: true,
                links_lost: false,
                annotations_lost: false,
                forms_lost: false,
                vector_quality_lost: true,
                color_space_fidelity_risk: false,
                spot_colors_lost: false,
                layers_lost: false,
                structure_association_lost: false,
                page_actions_lost: false,
            },
            blockers: vec![],
            warnings: vec![],
            executable: true,
            acknowledgements_satisfied: true,
            ready_for_execution: true,
        };

        // Page index out of total_pages bounds
        let mut bad_page = valid_plan.clone();
        bad_page.page_plans[0].page_index = 5;
        assert!(validate_secure_redaction_plan(&bad_page).is_err());

        // Duplicate page plan index
        let mut dup_plan = valid_plan.clone();
        dup_plan.facts.total_pages = 2;
        dup_plan.page_plans.push(dup_plan.page_plans[0].clone());
        assert!(validate_secure_redaction_plan(&dup_plan).is_err());

        // Page facts index mismatch
        let mut mismatch_facts = valid_plan.clone();
        mismatch_facts.page_plans[0].facts.page_index = 1;
        assert!(validate_secure_redaction_plan(&mismatch_facts).is_err());

        // Page geometry index mismatch
        let mut mismatch_geom = valid_plan.clone();
        mismatch_geom.page_plans[0].geometry.page_index = 1;
        assert!(validate_secure_redaction_plan(&mismatch_geom).is_err());

        // Region page index mismatch
        let mut mismatch_region = valid_plan.clone();
        mismatch_region.page_plans[0].canonical_regions[0].page_index = 1;
        assert!(validate_secure_redaction_plan(&mismatch_region).is_err());

        // Executable=true with blocker
        let mut blocker_exec = valid_plan.clone();
        blocker_exec.blockers.push(RedactionBlocker {
            schema_version: "1.0.0".to_string(),
            code: "BLOCK_1".to_string(),
            message: "msg".to_string(),
            details: None,
        });
        assert!(validate_secure_redaction_plan(&blocker_exec).is_err());
    }

    #[test]
    fn test_device_pixel_rect_arithmetic_overflow_and_boundaries() {
        assert!(validate_device_pixel_rect(&DevicePixelRect {
            left: 0,
            top: 0,
            width: 1,
            height: 1
        })
        .is_ok());

        assert!(validate_device_pixel_rect(&DevicePixelRect {
            left: -1,
            top: 0,
            width: 1,
            height: 1
        })
        .is_ok());

        assert!(validate_device_pixel_rect(&DevicePixelRect {
            left: i32::MIN,
            top: 0,
            width: 1,
            height: 1
        })
        .is_ok());

        assert!(validate_device_pixel_rect(&DevicePixelRect {
            left: i32::MAX,
            top: 0,
            width: 0,
            height: 1
        })
        .is_err());

        assert!(validate_device_pixel_rect(&DevicePixelRect {
            left: i32::MAX,
            top: 0,
            width: 1,
            height: 1
        })
        .is_err());
    }

    #[test]
    fn test_snake_case_wire_format_serde_keys() {
        let page_plan = sample_valid_page_plan();
        let plan_json = serde_json::to_value(&page_plan).expect("Serialize PageRedactionPlan");

        assert!(plan_json.get("schema_version").is_some());
        assert!(plan_json.get("page_index").is_some());
        assert!(plan_json.get("strategy").is_some());
        assert!(plan_json.get("geometry").is_some());
        assert!(plan_json.get("geometry_digest").is_some());
        assert!(plan_json.get("canonical_regions").is_some());
        assert!(plan_json.get("feature_losses").is_some());
        assert!(plan_json.get("executable").is_some());

        assert!(plan_json.get("schemaVersion").is_none());
        assert!(plan_json.get("pageIndex").is_none());
        assert!(plan_json.get("geometryDigest").is_none());
        assert!(plan_json.get("canonicalRegions").is_none());

        let losses_json =
            serde_json::to_value(&page_plan.feature_losses).expect("Serialize losses");
        assert!(losses_json.get("text_selection_lost").is_some());
        assert!(losses_json.get("searchability_lost").is_some());
        assert!(losses_json.get("accessibility_tags_lost").is_some());
        assert!(losses_json.get("vector_quality_lost").is_some());
        assert!(losses_json.get("accessibilityLost").is_none());
    }

    #[test]
    fn test_v1_wire_fixture_is_rejected_by_explicit_v2_geometry_migration() {
        let fixture_str =
            include_str!("../tests/fixtures/contracts/secure-redaction-plan-wire-v1.json");
        let fixture: serde_json::Value =
            serde_json::from_str(fixture_str).expect("Parse canonical wire fixture");

        let legacy: SecureRedactionPlan =
            serde_json::from_value(fixture).expect("Deserialize legacy v1 wire fixture");
        assert!(validate_secure_redaction_plan(&legacy).is_err());

        let typed = canonical_wire_fixture_plan();
        validate_secure_redaction_plan(&typed).expect("Validate v2 canonical wire DTO");
        assert_eq!(typed.plan_version, SECURE_REDACTION_PLAN_VERSION);
    }

    #[test]
    fn test_page_plan_executable_invariants_and_blockers() {
        let mut page_plan = sample_valid_page_plan();
        page_plan.facts.complex_resources = InspectionFactState::Unknown;
        page_plan.blockers = evaluate_page_redaction_blockers(&page_plan.facts);

        // Page plan with blocker + executable: true must be rejected
        page_plan.executable = true;
        assert!(validate_page_redaction_plan(&page_plan, 1).is_err());

        // Page plan with blocker + executable: false is valid at page level
        page_plan.executable = false;
        assert!(validate_page_redaction_plan(&page_plan, 1).is_ok());
    }

    #[test]
    fn test_contract_manifest_parity_with_rust_constants() {
        let manifest_str =
            include_str!("../tests/fixtures/contracts/secure-redaction-contract-v1.json");
        let manifest: serde_json::Value =
            serde_json::from_str(manifest_str).expect("Valid manifest JSON");

        assert_eq!(manifest["schemaVersion"], REDACTION_SCHEMA_VERSION);
        assert_eq!(manifest["wireFormat"], "snake_case");

        let limits = &manifest["limits"];
        assert_eq!(
            limits["MAX_REGIONS_PER_INPUT"].as_u64().unwrap() as usize,
            MAX_REGIONS_PER_INPUT
        );
        assert_eq!(
            limits["MAX_REGION_ID_LEN"].as_u64().unwrap() as usize,
            MAX_REGION_ID_LEN
        );
        assert_eq!(
            limits["MAX_SOURCE_LEN"].as_u64().unwrap() as usize,
            MAX_SOURCE_LEN
        );
        assert_eq!(
            limits["MAX_SOURCE_REGION_IDS"].as_u64().unwrap() as usize,
            MAX_SOURCE_REGION_IDS
        );
        assert_eq!(
            limits["MAX_CANONICAL_REGIONS_PER_PAGE"].as_u64().unwrap() as usize,
            MAX_CANONICAL_REGIONS_PER_PAGE
        );
        assert_eq!(
            limits["MAX_PAGE_PLANS"].as_u64().unwrap() as usize,
            MAX_PAGE_PLANS
        );
        assert_eq!(
            limits["MAX_BLOCKERS"].as_u64().unwrap() as usize,
            MAX_BLOCKERS
        );
        assert_eq!(
            limits["MAX_WARNINGS"].as_u64().unwrap() as usize,
            MAX_WARNINGS
        );
        assert_eq!(
            limits["MAX_VERIFICATION_CHECKS"].as_u64().unwrap() as usize,
            MAX_VERIFICATION_CHECKS
        );
        assert_eq!(
            limits["MAX_VERIFICATION_CHECK_CODE_LEN"].as_u64().unwrap() as usize,
            MAX_VERIFICATION_CHECK_CODE_LEN
        );
        assert_eq!(
            limits["MAX_VERIFICATION_CHECK_MESSAGE_LEN"]
                .as_u64()
                .unwrap() as usize,
            MAX_VERIFICATION_CHECK_MESSAGE_LEN
        );
        assert_eq!(
            limits["MAX_VERIFICATION_CHECK_DETAILS_LEN"]
                .as_u64()
                .unwrap() as usize,
            MAX_VERIFICATION_CHECK_DETAILS_LEN
        );
        assert_eq!(
            limits["MIN_RASTER_DPI"].as_u64().unwrap() as u32,
            MIN_RASTER_DPI
        );
        assert_eq!(
            limits["MAX_RASTER_DPI"].as_u64().unwrap() as u32,
            MAX_RASTER_DPI
        );
        assert_eq!(
            limits["MIN_SAFETY_MARGIN_POINTS"].as_f64().unwrap(),
            MIN_SAFETY_MARGIN_POINTS
        );
        assert_eq!(
            limits["MAX_SAFETY_MARGIN_POINTS"].as_f64().unwrap(),
            MAX_SAFETY_MARGIN_POINTS
        );

        let error_codes: Vec<String> = manifest["errorCodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let rust_error_codes: Vec<String> = SecureRedactionErrorCode::ALL
            .iter()
            .map(|code| code.code_str().to_string())
            .collect();
        assert_eq!(error_codes, rust_error_codes);
        let unique_codes: HashSet<&String> = error_codes.iter().collect();
        assert_eq!(unique_codes.len(), 15);

        let serialized_strings = |values: Vec<serde_json::Value>| {
            values
                .into_iter()
                .map(|value| {
                    value
                        .as_str()
                        .expect("enum serializes to string")
                        .to_string()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            manifest["profile"].as_str().unwrap(),
            serde_json::to_value(RedactionProfile::SecureRasterizedPagesV1)
                .unwrap()
                .as_str()
                .unwrap()
        );
        assert_eq!(
            manifest["documentStrategies"].as_array().unwrap(),
            &[serde_json::to_value(RedactionStrategy::RasterizedPages).unwrap()]
        );
        assert_eq!(
            manifest["pageStrategies"].as_array().unwrap(),
            &[
                serde_json::to_value(PageRedactionStrategy::RasterizePage).unwrap(),
                serde_json::to_value(PageRedactionStrategy::ImportUnaffectedPage).unwrap(),
            ]
        );
        assert_eq!(
            manifest["rotations"].as_array().unwrap(),
            &[
                serde_json::to_value(PageRotation::Deg0).unwrap(),
                serde_json::to_value(PageRotation::Deg90).unwrap(),
                serde_json::to_value(PageRotation::Deg180).unwrap(),
                serde_json::to_value(PageRotation::Deg270).unwrap(),
            ]
        );
        assert!(serde_json::from_str::<PageRotation>("45").is_err());
        assert_eq!(
            manifest["verificationStatuses"].as_array().unwrap(),
            &[
                serde_json::to_value(VerificationCheckStatus::Passed).unwrap(),
                serde_json::to_value(VerificationCheckStatus::Failed).unwrap(),
                serde_json::to_value(VerificationCheckStatus::NotRun).unwrap(),
            ]
        );
        assert!(serde_json::from_str::<VerificationCheckStatus>("\"unknown\"").is_err());
        assert_eq!(
            serialized_strings(vec![
                serde_json::to_value(InteractiveContentPolicy::Strip).unwrap(),
                serde_json::to_value(InteractiveContentPolicy::Block).unwrap(),
            ]),
            manifest["interactiveContentPolicies"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            serialized_strings(vec![serde_json::to_value(SignaturePolicy::Block).unwrap()]),
            vec![manifest["signaturePolicies"][0]
                .as_str()
                .unwrap()
                .to_string()]
        );
        assert_eq!(
            serialized_strings(vec![serde_json::to_value(EncryptionPolicy::Block).unwrap()]),
            vec![manifest["encryptionPolicies"][0]
                .as_str()
                .unwrap()
                .to_string()]
        );
        assert_eq!(manifest["digest"]["algorithm"], "sha256");
        assert_eq!(manifest["digest"]["encoding"], "lowercase_hex");
        assert_eq!(manifest["digest"]["length"], 64);
        assert_eq!(manifest["stringLengthUnit"], "utf8_bytes");
        assert_eq!(manifest["numericRanges"]["u32Min"], u32::MIN);
        assert_eq!(manifest["numericRanges"]["u32Max"], u32::MAX);
        assert_eq!(manifest["numericRanges"]["i32Min"], i32::MIN);
        assert_eq!(manifest["numericRanges"]["i32Max"], i32::MAX);
    }

    #[test]
    fn test_digest_strict_validation_boundaries() {
        assert!(validate_sha256_digest(&"a".repeat(64), "digest").is_ok());
        assert!(validate_sha256_digest(&"a".repeat(63), "digest").is_err());
        assert!(validate_sha256_digest(&"a".repeat(65), "digest").is_err());
        assert!(validate_sha256_digest(&"A".repeat(64), "digest").is_err());
        assert!(validate_sha256_digest(&format!("{} ", "a".repeat(64)), "digest").is_err());
        assert!(validate_sha256_digest(&format!("{}z", "a".repeat(63)), "digest").is_err());
    }

    #[test]
    fn test_active_limits_use_utf8_bytes_and_exact_collection_boundaries() {
        let input = sample_valid_input(&"é".repeat(MAX_REGION_ID_LEN / 2), 0.0, 0.0, 0.5, 0.5);
        assert!(validate_redaction_region_input(&input).is_ok());
        let too_long_input = sample_valid_input(
            &format!("{}a", "é".repeat(MAX_REGION_ID_LEN / 2)),
            0.0,
            0.0,
            0.5,
            0.5,
        );
        assert!(validate_redaction_region_input(&too_long_input).is_err());

        let mut blocker = RedactionBlocker {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            code: "b".repeat(MAX_BLOCKER_CODE_LEN),
            message: "m".repeat(MAX_USER_SAFE_MESSAGE_LEN),
            details: Some("d".repeat(MAX_DETAILS_LEN)),
        };
        assert!(validate_blocker(&blocker).is_ok());
        blocker.code.push('x');
        assert!(validate_blocker(&blocker).is_err());

        let mut warning = RedactionWarning {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            code: "w".repeat(MAX_WARNING_CODE_LEN),
            message: "m".repeat(MAX_USER_SAFE_MESSAGE_LEN),
            details: Some("d".repeat(MAX_DETAILS_LEN)),
        };
        assert!(validate_warning(&warning).is_ok());
        warning.details.as_mut().unwrap().push('x');
        assert!(validate_warning(&warning).is_err());

        let mut check = RedactionVerificationCheck {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            code: "c".repeat(MAX_VERIFICATION_CHECK_CODE_LEN),
            status: VerificationCheckStatus::Passed,
            message: "m".repeat(MAX_VERIFICATION_CHECK_MESSAGE_LEN),
            details: Some("d".repeat(MAX_VERIFICATION_CHECK_DETAILS_LEN)),
        };
        assert!(validate_verification_check(&check).is_ok());
        check.message.push('x');
        assert!(validate_verification_check(&check).is_err());

        let publication = SecureRedactionPublicationReport {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            publication_id: "i".repeat(MAX_IDENTITY_LEN),
            published_path: "p".repeat(MAX_PATH_LEN),
            published_digest: sample_sha256(),
            commit_state: SecureRedactionCommitState::NewCommitted,
        };
        assert!(validate_publication_report(&publication).is_ok());
        let mut oversized_publication = publication.clone();
        oversized_publication.published_path.push('x');
        assert!(validate_publication_report(&oversized_publication).is_err());

        let blockers = (0..MAX_BLOCKERS)
            .map(|index| RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: format!("b{index}"),
                message: "m".to_string(),
                details: None,
            })
            .collect::<Vec<_>>();
        assert!(validate_unique_blockers(&blockers).is_ok());
        let mut too_many_blockers = blockers;
        too_many_blockers.push(RedactionBlocker {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            code: "extra".to_string(),
            message: "m".to_string(),
            details: None,
        });
        assert!(validate_unique_blockers(&too_many_blockers).is_err());

        for dpi in [MIN_RASTER_DPI, MAX_RASTER_DPI] {
            let mut policy = sample_valid_policy();
            policy.raster_dpi = dpi;
            assert!(validate_secure_redaction_policy(&policy).is_ok());
        }
        let mut invalid_policy = sample_valid_policy();
        invalid_policy.raster_dpi = MAX_RASTER_DPI + 1;
        assert!(validate_secure_redaction_policy(&invalid_policy).is_err());
        invalid_policy = sample_valid_policy();
        invalid_policy.safety_margin_points = MAX_SAFETY_MARGIN_POINTS + 0.001;
        assert!(validate_secure_redaction_policy(&invalid_policy).is_err());
    }

    #[test]
    fn test_execution_report_cross_field_invariants_and_publication() {
        let digest = sample_sha256();
        let mut transform_report = sample_transform_report();
        transform_report.plan_digest = digest.clone();
        transform_report.input_digest = digest.clone();
        transform_report.output_digest = digest.clone();

        let ver_report_unverified = SecureRedactionVerificationReport {
            schema_version: "1.0.0".to_string(),
            verification_version: SECURE_REDACTION_VERIFICATION_VERSION.to_string(),
            job_id: "job_1".to_string(),
            session_id: "sess_1".to_string(),
            plan_digest: digest.clone(),
            transform_digest: digest.clone(),
            is_verified: false,
            verification_checks: required_verification_checks(VerificationCheckStatus::Passed),
            blockers: vec![],
            warnings: vec![],
        };

        let pub_report = SecureRedactionPublicationReport {
            schema_version: "1.0.0".to_string(),
            publication_id: "pub_1".to_string(),
            published_path: "/published.pdf".to_string(),
            published_digest: digest.clone(),
            commit_state: SecureRedactionCommitState::NewCommitted,
        };

        let exec_invalid = SecureRedactionExecutionReport {
            schema_version: "1.0.0".to_string(),
            job_id: "job_1".to_string(),
            session_id: "sess_1".to_string(),
            transform_report: transform_report.clone(),
            verification_report: ver_report_unverified.clone(),
            published: true,
            publication_report: Some(pub_report.clone()),
        };
        assert!(validate_execution_report(&exec_invalid).is_err());

        let exec_invalid_pub = SecureRedactionExecutionReport {
            schema_version: "1.0.0".to_string(),
            job_id: "job_1".to_string(),
            session_id: "sess_1".to_string(),
            transform_report: transform_report.clone(),
            verification_report: ver_report_unverified.clone(),
            published: false,
            publication_report: Some(pub_report.clone()),
        };
        assert!(validate_execution_report(&exec_invalid_pub).is_err());

        let ver_report_valid = SecureRedactionVerificationReport {
            schema_version: "1.0.0".to_string(),
            verification_version: SECURE_REDACTION_VERIFICATION_VERSION.to_string(),
            job_id: "job_1".to_string(),
            session_id: "sess_1".to_string(),
            plan_digest: digest.clone(),
            transform_digest: digest.clone(),
            is_verified: true,
            verification_checks: required_verification_checks(VerificationCheckStatus::Passed),
            blockers: vec![],
            warnings: vec![],
        };
        let exec_valid = SecureRedactionExecutionReport {
            schema_version: "1.0.0".to_string(),
            job_id: "job_1".to_string(),
            session_id: "sess_1".to_string(),
            transform_report: transform_report.clone(),
            verification_report: ver_report_valid.clone(),
            published: true,
            publication_report: Some(pub_report),
        };
        assert!(validate_execution_report(&exec_valid).is_ok());
    }

    #[test]
    fn test_verification_report_is_verified_is_a_biconditional() {
        let verified = || SecureRedactionVerificationReport {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            verification_version: SECURE_REDACTION_VERIFICATION_VERSION.to_string(),
            job_id: "job_1".to_string(),
            session_id: "sess_1".to_string(),
            plan_digest: sample_sha256(),
            transform_digest: sample_sha256(),
            is_verified: true,
            verification_checks: required_verification_checks(VerificationCheckStatus::Passed),
            blockers: vec![],
            warnings: vec![],
        };
        assert!(validate_verification_report(&verified()).is_ok());

        let mut false_negative = verified();
        false_negative.is_verified = false;
        assert!(validate_verification_report(&false_negative).is_err());

        let mut failed = verified();
        failed.is_verified = false;
        failed.verification_checks[0].status = VerificationCheckStatus::Failed;
        failed.blockers.push(RedactionBlocker {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            code: "REDACTION_VERIFICATION_TEST_FAILED".to_string(),
            message: "failed".to_string(),
            details: None,
        });
        assert!(validate_verification_report(&failed).is_ok());

        let mut not_run = failed.clone();
        not_run.verification_checks[1].status = VerificationCheckStatus::NotRun;
        assert!(validate_verification_report(&not_run).is_ok());

        let mut invalid_true = failed;
        invalid_true.is_verified = true;
        assert!(validate_verification_report(&invalid_true).is_err());
    }

    #[test]
    fn test_utf8_byte_length_unicode_parity() {
        let turkish_str = "ğüşiöçĞÜŞİÖÇ";
        assert_eq!(turkish_str.len(), 23);
        assert_ne!(turkish_str.chars().count(), turkish_str.len());

        let emoji_str = "🔐🔒";
        assert_eq!(emoji_str.len(), 8);
    }

    #[test]
    fn test_contract_manifest_appearance_matches_rust_runtime() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/contracts/secure-redaction-contract-v1.json"
        ))
        .expect("parse contract manifest");
        let appearance = &manifest["appearance"];

        let serialized = serde_json::to_value(&sample_valid_policy().appearance)
            .expect("serialize runtime appearance");
        assert_eq!(serialized["kind"], appearance["kinds"][0]);
        assert_eq!(serialized["color"], appearance["colors"][0]);
        assert_eq!(serialized["opacity"], appearance["opacity"][0]);

        for kind in appearance["kinds"].as_array().expect("appearance kinds") {
            for color in appearance["colors"].as_array().expect("appearance colors") {
                for opacity in appearance["opacity"]
                    .as_array()
                    .expect("appearance opacity")
                {
                    let mut policy = sample_valid_policy();
                    policy.appearance = serde_json::from_value(serde_json::json!({
                        "kind": kind,
                        "color": color,
                        "opacity": opacity,
                    }))
                    .expect("manifest appearance deserializes");
                    assert!(validate_secure_redaction_policy(&policy).is_ok());
                }
            }
        }

        for (kind, color, opacity) in [
            ("unknown", "black", 1.0),
            ("solid", "unknown", 1.0),
            ("solid", "black", 0.999_999),
            ("solid", "black", 1.000_001),
            ("solid", "black", f64::NAN),
            ("solid", "black", f64::INFINITY),
            ("solid", "black", f64::NEG_INFINITY),
        ] {
            let mut policy = sample_valid_policy();
            policy.appearance = RedactionAppearancePolicy {
                kind: kind.to_string(),
                color: color.to_string(),
                opacity,
            };
            assert!(validate_secure_redaction_policy(&policy).is_err());
        }
    }

    #[test]
    fn test_unsupported_schema_versions_fail_closed_for_versioned_rust_dtos() {
        fn assert_unsupported(
            result: Result<(), crate::document_session::SessionJobOperationError>,
        ) {
            match result.expect_err("unsupported schema version must fail") {
                crate::document_session::SessionJobOperationError::Failed(error) => {
                    assert_eq!(error.code, "REDACTION_POLICY_VERSION_UNSUPPORTED");
                }
                other => panic!("expected typed failed validation error, got {other:?}"),
            }
        }

        let mut policy = sample_valid_policy();
        policy.schema_version = "999.0.0".to_string();
        assert_unsupported(validate_secure_redaction_policy(&policy));
        let mut facts = canonical_wire_fixture_plan().facts;
        facts.schema_version = "999.0.0".to_string();
        assert_unsupported(validate_document_redaction_facts(&facts));
        let mut page_facts = sample_valid_page_plan().facts;
        page_facts.schema_version = "999.0.0".to_string();
        assert_unsupported(validate_page_redaction_facts(&page_facts, 1));
        let mut losses = sample_valid_page_plan().feature_losses;
        losses.schema_version = "999.0.0".to_string();
        assert_unsupported(validate_redaction_feature_losses(&losses));
        let blocker = RedactionBlocker {
            schema_version: "999.0.0".to_string(),
            code: "B".to_string(),
            message: "block".to_string(),
            details: None,
        };
        assert_unsupported(validate_blocker(&blocker));
        let warning = RedactionWarning {
            schema_version: "999.0.0".to_string(),
            code: "W".to_string(),
            message: "warn".to_string(),
            details: None,
        };
        assert_unsupported(validate_warning(&warning));
        let check = RedactionVerificationCheck {
            schema_version: "999.0.0".to_string(),
            code: "C".to_string(),
            status: VerificationCheckStatus::Passed,
            message: "passed".to_string(),
            details: None,
        };
        assert_unsupported(validate_verification_check(&check));
        let mut page_plan = sample_valid_page_plan();
        page_plan.schema_version = "999.0.0".to_string();
        assert_unsupported(validate_page_redaction_plan(&page_plan, 1));
        let mut plan = canonical_wire_fixture_plan();
        plan.schema_version = "999.0.0".to_string();
        assert_unsupported(validate_secure_redaction_plan(&plan));

        let mut transform = sample_transform_report();
        transform.schema_version = "999.0.0".to_string();
        transform.job_id = "job".to_string();
        transform.session_id = "session".to_string();
        assert_unsupported(validate_transform_report(&transform));
        transform.schema_version = REDACTION_SCHEMA_VERSION.to_string();
        let mut verification = SecureRedactionVerificationReport {
            schema_version: "999.0.0".to_string(),
            verification_version: SECURE_REDACTION_VERIFICATION_VERSION.to_string(),
            job_id: "job".to_string(),
            session_id: "session".to_string(),
            plan_digest: sample_sha256(),
            transform_digest: sample_sha256(),
            is_verified: true,
            verification_checks: vec![RedactionVerificationCheck {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "C".to_string(),
                status: VerificationCheckStatus::Passed,
                message: "passed".to_string(),
                details: None,
            }],
            blockers: vec![],
            warnings: vec![],
        };
        assert_unsupported(validate_verification_report(&verification));
        verification.schema_version = REDACTION_SCHEMA_VERSION.to_string();
        let mut publication = SecureRedactionPublicationReport {
            schema_version: "999.0.0".to_string(),
            publication_id: "publication".to_string(),
            published_path: "/published.pdf".to_string(),
            published_digest: sample_sha256(),
            commit_state: SecureRedactionCommitState::NewCommitted,
        };
        assert_unsupported(validate_publication_report(&publication));
        publication.schema_version = REDACTION_SCHEMA_VERSION.to_string();
        let execution = SecureRedactionExecutionReport {
            schema_version: "999.0.0".to_string(),
            job_id: "job".to_string(),
            session_id: "session".to_string(),
            transform_report: transform,
            verification_report: verification,
            published: false,
            publication_report: None,
        };
        assert_unsupported(validate_execution_report(&execution));
    }

    #[test]
    fn test_all_active_collection_limits_accept_exact_and_reject_one_over() {
        let inputs = (0..MAX_REGIONS_PER_INPUT)
            .map(|index| sample_valid_input(&format!("region-{index}"), 0.0, 0.0, 0.5, 0.5))
            .collect::<Vec<_>>();
        assert!(validate_redaction_region_inputs(&inputs).is_ok());
        let mut too_many_inputs = inputs;
        too_many_inputs.push(sample_valid_input("extra", 0.0, 0.0, 0.5, 0.5));
        assert!(validate_redaction_region_inputs(&too_many_inputs).is_err());

        let mut region = sample_valid_page_plan().canonical_regions.remove(0);
        region.source_region_ids = (0..MAX_SOURCE_REGION_IDS)
            .map(|index| format!("source-{index:03}"))
            .collect();
        assert!(validate_canonical_redaction_region(&region).is_ok());
        region.source_region_ids.push("source-999".to_string());
        assert!(validate_canonical_redaction_region(&region).is_err());

        let mut page_plan = sample_valid_page_plan();
        page_plan.canonical_regions = (0..MAX_CANONICAL_REGIONS_PER_PAGE)
            .map(|index| CanonicalRedactionRegion {
                region_id: format!("region-{index:04}"),
                ..sample_valid_page_plan().canonical_regions[0].clone()
            })
            .collect();
        assert!(validate_page_redaction_plan(&page_plan, 1).is_ok());
        page_plan.canonical_regions.push(CanonicalRedactionRegion {
            region_id: "extra".to_string(),
            ..sample_valid_page_plan().canonical_regions[0].clone()
        });
        assert!(validate_page_redaction_plan(&page_plan, 1).is_err());

        let blockers = (0..MAX_BLOCKERS)
            .map(|index| RedactionBlocker {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: format!("B{index}"),
                message: "message".to_string(),
                details: None,
            })
            .collect::<Vec<_>>();
        assert!(validate_unique_blockers(&blockers).is_ok());
        let mut too_many_blockers = blockers;
        too_many_blockers.push(RedactionBlocker {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            code: "extra".to_string(),
            message: "message".to_string(),
            details: None,
        });
        assert!(validate_unique_blockers(&too_many_blockers).is_err());

        let warnings = (0..MAX_WARNINGS)
            .map(|index| RedactionWarning {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: format!("W{index}"),
                message: "message".to_string(),
                details: None,
            })
            .collect::<Vec<_>>();
        assert!(validate_unique_warnings(&warnings).is_ok());
        let mut too_many_warnings = warnings;
        too_many_warnings.push(RedactionWarning {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            code: "extra".to_string(),
            message: "message".to_string(),
            details: None,
        });
        assert!(validate_unique_warnings(&too_many_warnings).is_err());

        let checks = (0..MAX_VERIFICATION_CHECKS)
            .map(|index| RedactionVerificationCheck {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: format!("C{index}"),
                status: VerificationCheckStatus::Passed,
                message: "passed".to_string(),
                details: None,
            })
            .collect::<Vec<_>>();
        assert!(validate_unique_verification_checks(&checks).is_ok());
        let mut too_many_checks = checks;
        too_many_checks.push(RedactionVerificationCheck {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            code: "extra".to_string(),
            status: VerificationCheckStatus::Passed,
            message: "passed".to_string(),
            details: None,
        });
        assert!(validate_unique_verification_checks(&too_many_checks).is_err());

        let mut plan = canonical_wire_fixture_plan();
        plan.policy.raster_dpi = 72;
        plan.facts.total_pages = MAX_PAGE_PLANS as u32;
        plan.page_plans = (0..MAX_PAGE_PLANS)
            .map(|index| {
                let mut page = sample_valid_page_plan();
                page.page_index = index as u32;
                page.geometry.page_index = index as u32;
                page.geometry_digest = compute_geometry_digest(&page.geometry);
                page.facts.page_index = index as u32;
                page.canonical_regions[0].page_index = index as u32;
                page.canonical_regions[0].region_id = format!("canon_p{index}_s1");
                page
            })
            .collect();
        plan.canonical_regions = plan
            .page_plans
            .iter()
            .flat_map(|p| p.canonical_regions.clone())
            .collect();
        plan.acknowledgement_requirements = derive_redaction_acknowledgement_requirements(
            &plan.facts,
            &plan.losses,
            &plan.page_plans,
        );
        plan.acknowledgements.rasterized_pages_accepted = true;
        plan.acknowledgements_satisfied = evaluate_acknowledgements_satisfied(
            &plan.acknowledgement_requirements,
            &plan.acknowledgements,
        );
        plan.ready_for_execution = plan.executable && plan.acknowledgements_satisfied;
        plan.plan_digest = compute_plan_digest(
            &plan.input_digest,
            plan.content_generation,
            &plan.policy,
            &plan.acknowledgements,
            &plan.acknowledgement_requirements,
            &plan.facts,
            &plan.page_plans,
            &plan.canonical_regions,
            &plan.losses,
            &plan.blockers,
            &plan.warnings,
            plan.executable,
            plan.acknowledgements_satisfied,
            plan.ready_for_execution,
        );
        validate_secure_redaction_plan(&plan).unwrap();
        plan.page_plans.push(sample_valid_page_plan());
        assert!(validate_secure_redaction_plan(&plan).is_err());
    }

    #[test]
    fn test_all_active_string_limits_use_exact_utf8_bytes_and_reject_one_over() {
        fn utf8_exact(limit: usize) -> String {
            let value = format!("aa{}éğ🔒", "é".repeat((limit - 10) / 2));
            assert_eq!(value.as_bytes().len(), limit);
            value
        }
        assert_eq!("ascii".chars().count(), "ascii".as_bytes().len());
        for value in ["Türkçe", "é", "🔒", "aéğ🔒"] {
            assert_ne!(value.chars().count(), value.as_bytes().len());
        }
        let exact_region = utf8_exact(MAX_REGION_ID_LEN);
        assert!(validate_redaction_region_input(&sample_valid_input(
            &exact_region,
            0.0,
            0.0,
            0.5,
            0.5
        ))
        .is_ok());
        assert!(validate_redaction_region_input(&sample_valid_input(
            &format!("{exact_region}a"),
            0.0,
            0.0,
            0.5,
            0.5
        ))
        .is_err());
        let exact_source = utf8_exact(MAX_SOURCE_LEN);
        let mut input = sample_valid_input("region", 0.0, 0.0, 0.5, 0.5);
        input.source = exact_source.clone();
        assert!(validate_redaction_region_input(&input).is_ok());
        input.source.push('a');
        assert!(validate_redaction_region_input(&input).is_err());

        let blocker = RedactionBlocker {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            code: utf8_exact(MAX_BLOCKER_CODE_LEN),
            message: utf8_exact(MAX_USER_SAFE_MESSAGE_LEN),
            details: Some(utf8_exact(MAX_DETAILS_LEN)),
        };
        assert!(validate_blocker(&blocker).is_ok());
        for oversize in [
            |value: &mut RedactionBlocker| value.code.push('a'),
            |value: &mut RedactionBlocker| value.message.push('a'),
            |value: &mut RedactionBlocker| value.details.as_mut().expect("details").push('a'),
        ] {
            let mut invalid = blocker.clone();
            oversize(&mut invalid);
            assert!(validate_blocker(&invalid).is_err());
        }
        let warning = RedactionWarning {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            code: utf8_exact(MAX_WARNING_CODE_LEN),
            message: utf8_exact(MAX_USER_SAFE_MESSAGE_LEN),
            details: Some(utf8_exact(MAX_DETAILS_LEN)),
        };
        assert!(validate_warning(&warning).is_ok());
        for oversize in [
            |value: &mut RedactionWarning| value.code.push('a'),
            |value: &mut RedactionWarning| value.message.push('a'),
            |value: &mut RedactionWarning| value.details.as_mut().expect("details").push('a'),
        ] {
            let mut invalid = warning.clone();
            oversize(&mut invalid);
            assert!(validate_warning(&invalid).is_err());
        }
        let check = RedactionVerificationCheck {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            code: utf8_exact(MAX_VERIFICATION_CHECK_CODE_LEN),
            status: VerificationCheckStatus::Passed,
            message: utf8_exact(MAX_VERIFICATION_CHECK_MESSAGE_LEN),
            details: Some(utf8_exact(MAX_VERIFICATION_CHECK_DETAILS_LEN)),
        };
        assert!(validate_verification_check(&check).is_ok());
        for oversize in [
            |value: &mut RedactionVerificationCheck| value.code.push('a'),
            |value: &mut RedactionVerificationCheck| value.message.push('a'),
            |value: &mut RedactionVerificationCheck| {
                value.details.as_mut().expect("details").push('a')
            },
        ] {
            let mut invalid = check.clone();
            oversize(&mut invalid);
            assert!(validate_verification_check(&invalid).is_err());
        }

        let publication = SecureRedactionPublicationReport {
            schema_version: REDACTION_SCHEMA_VERSION.to_string(),
            publication_id: utf8_exact(MAX_IDENTITY_LEN),
            published_path: utf8_exact(MAX_PATH_LEN),
            published_digest: sample_sha256(),
            commit_state: SecureRedactionCommitState::NewCommitted,
        };
        assert!(validate_publication_report(&publication).is_ok());
        for oversize in [
            |value: &mut SecureRedactionPublicationReport| value.publication_id.push('a'),
            |value: &mut SecureRedactionPublicationReport| value.published_path.push('a'),
        ] {
            let mut invalid = publication.clone();
            oversize(&mut invalid);
            assert!(validate_publication_report(&invalid).is_err());
        }
        let mut transform = sample_transform_report();
        transform.job_id = utf8_exact(MAX_IDENTITY_LEN);
        transform.session_id = utf8_exact(MAX_IDENTITY_LEN);
        assert!(validate_transform_report(&transform).is_ok());
        for oversize in [
            |value: &mut SecureRedactionTransformReport| value.job_id.push('a'),
            |value: &mut SecureRedactionTransformReport| value.session_id.push('a'),
            |value: &mut SecureRedactionTransformReport| value.output_relative_name.push('a'),
        ] {
            let mut invalid = transform.clone();
            oversize(&mut invalid);
            assert!(validate_transform_report(&invalid).is_err());
        }
    }

    #[test]
    fn test_numeric_policy_limits_accept_min_and_max_and_reject_each_side() {
        for raster_dpi in [MIN_RASTER_DPI, MAX_RASTER_DPI] {
            let mut policy = sample_valid_policy();
            policy.raster_dpi = raster_dpi;
            assert!(validate_secure_redaction_policy(&policy).is_ok());
        }
        for raster_dpi in [MIN_RASTER_DPI - 1, MAX_RASTER_DPI + 1] {
            let mut policy = sample_valid_policy();
            policy.raster_dpi = raster_dpi;
            assert!(validate_secure_redaction_policy(&policy).is_err());
        }
        for safety_margin_points in [MIN_SAFETY_MARGIN_POINTS, MAX_SAFETY_MARGIN_POINTS] {
            let mut policy = sample_valid_policy();
            policy.safety_margin_points = safety_margin_points;
            assert!(validate_secure_redaction_policy(&policy).is_ok());
        }
        for safety_margin_points in [
            MIN_SAFETY_MARGIN_POINTS - 0.001,
            MAX_SAFETY_MARGIN_POINTS + 0.001,
        ] {
            let mut policy = sample_valid_policy();
            policy.safety_margin_points = safety_margin_points;
            assert!(validate_secure_redaction_policy(&policy).is_err());
        }
    }

    #[test]
    fn test_transform_report_semantic_partition_evidence_and_size_limits() {
        assert!(validate_transform_report(&sample_populated_transform_report()).is_ok());

        let mut cases: Vec<(&str, Box<dyn Fn(&mut SecureRedactionTransformReport)>)> = vec![
            (
                "duplicate affected page",
                Box::new(|r| r.affected_pages.push(0)),
            ),
            (
                "duplicate unaffected page",
                Box::new(|r| {
                    r.page_count = 2;
                    r.unaffected_pages = vec![1, 1];
                    r.page_feature_losses.push(r.page_feature_losses[0].clone());
                }),
            ),
            (
                "affected-unaffected overlap",
                Box::new(|r| {
                    r.unaffected_pages = vec![0];
                    r.page_count = 2;
                    r.page_feature_losses.push(r.page_feature_losses[0].clone());
                }),
            ),
            (
                "missing partition page",
                Box::new(|r| {
                    r.page_count = 2;
                    r.page_feature_losses.push(r.page_feature_losses[0].clone());
                }),
            ),
            ("out-of-range page", Box::new(|r| r.affected_pages[0] = 1)),
            (
                "duplicate raster evidence",
                Box::new(|r| {
                    r.affected_page_raster_geometry
                        .push(r.affected_page_raster_geometry[0].clone())
                }),
            ),
            (
                "missing raster evidence",
                Box::new(|r| r.affected_page_raster_geometry.clear()),
            ),
            (
                "raster evidence on unaffected page",
                Box::new(|r| {
                    r.page_count = 2;
                    r.unaffected_pages = vec![1];
                    r.page_feature_losses.push(TransformPageFeatureLoss {
                        page_index: 1,
                        feature_losses: r.page_feature_losses[0].feature_losses.clone(),
                    });
                    r.affected_page_raster_geometry[0].page_index = 1;
                }),
            ),
            (
                "duplicate feature-loss evidence",
                Box::new(|r| r.page_feature_losses.push(r.page_feature_losses[0].clone())),
            ),
            (
                "missing feature-loss evidence",
                Box::new(|r| r.page_feature_losses.clear()),
            ),
            (
                "region on unaffected page",
                Box::new(|r| r.applied_canonical_regions[0].page_index = 1),
            ),
            (
                "oversized raster dimensions",
                Box::new(|r| r.affected_page_raster_geometry[0].width = MAX_RASTER_DIMENSION + 1),
            ),
            ("zero output size", Box::new(|r| r.output_size_bytes = 0)),
            (
                "oversized output size",
                Box::new(|r| r.output_size_bytes = MAX_SECURE_REDACTION_OUTPUT_BYTES + 1),
            ),
        ];
        for (name, mutate) in cases.drain(..) {
            let mut invalid = sample_populated_transform_report();
            mutate(&mut invalid);
            assert!(
                validate_transform_report(&invalid).is_err(),
                "{name} must be rejected"
            );
        }

        let mut too_large = sample_transform_report();
        too_large.job_id = "x".repeat(MAX_REPORT_SIZE_BYTES + 1);
        let error = validate_transform_report(&too_large).unwrap_err();
        match error {
            SessionJobOperationError::Failed(failure) => {
                assert_eq!(failure.code, "REDACTION_RESOURCE_LIMIT_EXCEEDED")
            }
            SessionJobOperationError::Cancelled(_) => panic!("report validation cannot cancel"),
        }
    }

    // -----------------------------------------------------------------------
    // Prompt 8B-3 Tests
    // -----------------------------------------------------------------------

    fn sample_planner_input_2pages() -> RedactionPlannerInput {
        let geom0 = sample_valid_geometry();
        let mut geom1 = sample_valid_geometry();
        geom1.page_index = 1;

        let input_r0 = sample_valid_input("r0_1", 0.1, 0.1, 0.2, 0.2);

        RedactionPlannerInput {
            input_digest: sample_sha256(),
            content_generation: 1,
            policy: sample_valid_policy(),
            acknowledgements: sample_valid_acknowledgements(),
            region_inputs: vec![input_r0],
            page_geometries: vec![geom0, geom1],
            document_facts: DocumentRedactionFacts {
                schema_version: "1.0.0".to_string(),
                total_pages: 2,
                encryption: InspectionFactState::Absent,
                signature_dictionary: InspectionFactState::Absent,
                signature_field: InspectionFactState::Absent,
                byte_range: InspectionFactState::Absent,
                acro_form: InspectionFactState::Absent,
                xfa: InspectionFactState::Absent,
                annotations: InspectionFactState::Absent,
                optional_content: InspectionFactState::Absent,
                embedded_files: InspectionFactState::Absent,
                javascript: InspectionFactState::Absent,
                incremental_revisions: InspectionFactState::Absent,
                structure_tree: InspectionFactState::Absent,
                outlines: InspectionFactState::Absent,
                names_dictionary: InspectionFactState::Absent,
                document_actions: InspectionFactState::Absent,
                complex_document_resources: InspectionFactState::Absent,
            },
            page_facts: vec![
                PageRedactionFacts {
                    schema_version: "1.0.0".to_string(),
                    page_index: 0,
                    text: InspectionFactState::Present,
                    images: InspectionFactState::Absent,
                    vector_paths: InspectionFactState::Absent,
                    form_xobjects: InspectionFactState::Absent,
                    annotations: InspectionFactState::Absent,
                    link_annotations: InspectionFactState::Absent,
                    widgets: InspectionFactState::Absent,
                    transparency: InspectionFactState::Absent,
                    optional_content: InspectionFactState::Absent,
                    complex_resources: InspectionFactState::Absent,
                    structure_association: InspectionFactState::Absent,
                    page_actions: InspectionFactState::Absent,
                    spot_colors: InspectionFactState::Absent,
                    complex_color_spaces: InspectionFactState::Absent,
                    geometry_profile: GeometryProfile::StandardEffectiveBoxV1,
                    rotation: PageRotation::Deg0,
                },
                PageRedactionFacts {
                    schema_version: "1.0.0".to_string(),
                    page_index: 1,
                    text: InspectionFactState::Present,
                    images: InspectionFactState::Present,
                    vector_paths: InspectionFactState::Absent,
                    form_xobjects: InspectionFactState::Absent,
                    annotations: InspectionFactState::Absent,
                    link_annotations: InspectionFactState::Absent,
                    widgets: InspectionFactState::Absent,
                    transparency: InspectionFactState::Absent,
                    optional_content: InspectionFactState::Absent,
                    complex_resources: InspectionFactState::Absent,
                    structure_association: InspectionFactState::Absent,
                    page_actions: InspectionFactState::Absent,
                    spot_colors: InspectionFactState::Absent,
                    complex_color_spaces: InspectionFactState::Absent,
                    geometry_profile: GeometryProfile::StandardEffectiveBoxV1,
                    rotation: PageRotation::Deg0,
                },
            ],
        }
    }

    #[test]
    fn test_planner_valid_plan_and_affected_unaffected_pages() {
        let input = sample_planner_input_2pages();
        let plan = build_secure_redaction_plan(&input).expect("Build plan success");

        assert!(plan.executable);
        assert_eq!(plan.page_plans.len(), 2);

        // Page 0 (Affected)
        assert_eq!(plan.page_plans[0].canonical_regions.len(), 1);
        assert!(plan.page_plans[0].feature_losses.text_selection_lost);
        assert!(!plan.page_plans[0].feature_losses.vector_quality_lost);

        // Page 1 (Unaffected)
        assert_eq!(plan.page_plans[1].canonical_regions.len(), 0);
        assert!(!plan.page_plans[1].feature_losses.text_selection_lost);
        assert!(!plan.page_plans[1].feature_losses.vector_quality_lost);

        // Document-level losses aggregated
        assert!(plan.losses.text_selection_lost);
    }

    #[test]
    fn test_rebuild_geometry_capability_is_planner_worker_parity_gate() {
        let mut supported = sample_planner_input_2pages();
        supported.page_geometries[0].media_box.left = -100.0;
        supported.page_geometries[0].media_box.bottom = 50.0;
        supported.page_geometries[0].effective_box = supported.page_geometries[0].media_box.clone();
        supported.page_geometries[0].crop_box = Some(PdfPointRect {
            left: -50.0,
            bottom: 75.0,
            width: 400.0,
            height: 600.0,
        });
        supported.page_geometries[0].effective_box =
            supported.page_geometries[0].crop_box.clone().unwrap();
        supported.page_geometries[0].trim_box = Some(PdfPointRect {
            left: -80.0,
            bottom: 60.0,
            width: 500.0,
            height: 700.0,
        });
        supported.page_geometries[0].bleed_box = Some(PdfPointRect {
            left: -90.0,
            bottom: 55.0,
            width: 520.0,
            height: 720.0,
        });
        supported.page_geometries[0].art_box = Some(PdfPointRect {
            left: -70.0,
            bottom: 80.0,
            width: 350.0,
            height: 500.0,
        });
        supported.page_geometries[0].rotation = PageRotation::Deg270;
        supported.page_facts[0].rotation = PageRotation::Deg270;

        let plan =
            build_secure_redaction_plan(&supported).expect("planner accepts safe PDFium geometry");
        assert!(plan.executable && plan.ready_for_execution);
        // This is precisely the worker's defense-in-depth call. An executable
        // plan cannot fail it solely because of geometry capability.
        evaluate_secure_redaction_rebuild_geometry(&plan.page_plans[0].geometry).unwrap();

        let mut unsupported = sample_planner_input_2pages();
        unsupported.page_geometries[0].media_box.left = 100_000_000.0;
        unsupported.page_geometries[0].effective_box.left = 100_000_000.0;
        let plan = build_secure_redaction_plan(&unsupported)
            .expect("planner returns blockers, not a late worker surprise");
        assert!(!plan.executable && !plan.ready_for_execution);
        assert!(plan.page_plans[0]
            .blockers
            .iter()
            .any(|blocker| { blocker.code == "REDACTION_PAGE_GEOMETRY_MISMATCH" }));
    }

    #[test]
    fn test_planner_blockers_encryption_signature_xfa_acroform() {
        let mut input = sample_planner_input_2pages();

        // Encrypted
        input.document_facts.encryption = InspectionFactState::Present;
        let plan_enc = build_secure_redaction_plan(&input).expect("Plan generated with blocker");
        assert!(!plan_enc.executable);
        assert!(plan_enc
            .blockers
            .iter()
            .any(|b| b.code == "REDACTION_ENCRYPTED_INPUT_UNSUPPORTED"));

        // Signed
        input.document_facts.encryption = InspectionFactState::Absent;
        input.document_facts.signature_dictionary = InspectionFactState::Present;
        let plan_signed = build_secure_redaction_plan(&input).expect("Plan generated with blocker");
        assert!(!plan_signed.executable);
        assert!(plan_signed
            .blockers
            .iter()
            .any(|b| b.code == "REDACTION_SIGNED_INPUT_UNSUPPORTED"));

        // XFA
        input.document_facts.signature_dictionary = InspectionFactState::Absent;
        input.document_facts.xfa = InspectionFactState::Present;
        let plan_xfa = build_secure_redaction_plan(&input).expect("Plan generated with blocker");
        assert!(!plan_xfa.executable);
        assert!(plan_xfa
            .blockers
            .iter()
            .any(|b| b.code == "REDACTION_XFA_INPUT_UNSUPPORTED"));

        // AcroForm
        input.document_facts.xfa = InspectionFactState::Absent;
        input.document_facts.acro_form = InspectionFactState::Present;
        input.policy.interactive_content_policy = InteractiveContentPolicy::Block;
        let plan_acro = build_secure_redaction_plan(&input).expect("Plan generated with blocker");
        assert!(!plan_acro.executable);
        assert!(plan_acro
            .blockers
            .iter()
            .any(|b| b.code == "REDACTION_BLOCKED_DOCUMENT_FEATURE"));
    }

    #[test]
    fn test_planner_geometry_rotations_and_coordinate_transform() {
        let norm = NormalizedTopLeftRect {
            x: 0.1,
            y: 0.1,
            width: 0.2,
            height: 0.2,
        };

        for rotation in [
            PageRotation::Deg0,
            PageRotation::Deg90,
            PageRotation::Deg180,
            PageRotation::Deg270,
        ] {
            let mut geom = sample_valid_geometry();
            geom.rotation = rotation;

            let pdf_rect = normalized_top_left_to_pdf_point_rect(&norm, &geom)
                .expect("Transform rect for rotation");
            assert!(pdf_rect.left.is_finite());
            assert!(pdf_rect.bottom.is_finite());
            assert!(pdf_rect.width > 0.0);
            assert!(pdf_rect.height > 0.0);
        }
    }

    #[test]
    fn test_planner_geometry_cropbox_and_non_zero_origin() {
        let norm = NormalizedTopLeftRect {
            x: 0.0,
            y: 0.0,
            width: 0.5,
            height: 0.5,
        };

        let mut geom = sample_valid_geometry();
        geom.media_box = PdfPointRect {
            left: -100.0,
            bottom: -100.0,
            width: 800.0,
            height: 1000.0,
        };
        geom.crop_box = Some(PdfPointRect {
            left: 50.0,
            bottom: 50.0,
            width: 500.0,
            height: 700.0,
        });
        geom.effective_box = PdfPointRect {
            left: 50.0,
            bottom: 50.0,
            width: 500.0,
            height: 700.0,
        };

        let pdf_rect = normalized_top_left_to_pdf_point_rect(&norm, &geom)
            .expect("Transform rect with offset origin");

        // Top left of effective box (50, 50, 500, 700) -> top is 750
        assert_eq!(pdf_rect.left, 50.0);
        assert_eq!(pdf_rect.width, 250.0);
        assert_eq!(pdf_rect.bottom, 400.0);
        assert_eq!(pdf_rect.height, 350.0);
    }

    #[test]
    fn test_planner_region_merge_determinism_and_order_independence() {
        let r1 = sample_valid_input("r1", 0.1, 0.1, 0.2, 0.2);
        let r2 = sample_valid_input("r2", 0.15, 0.15, 0.2, 0.2);

        let mut input_forward = sample_planner_input_2pages();
        input_forward.region_inputs = vec![r1.clone(), r2.clone()];

        let mut input_reversed = sample_planner_input_2pages();
        input_reversed.region_inputs = vec![r2, r1];

        let plan_forward = build_secure_redaction_plan(&input_forward).expect("Plan forward");
        let plan_reversed = build_secure_redaction_plan(&input_reversed).expect("Plan reversed");

        assert_eq!(plan_forward.page_plans[0].canonical_regions.len(), 1);
        assert_eq!(plan_reversed.page_plans[0].canonical_regions.len(), 1);
        assert_eq!(
            plan_forward.page_plans[0].canonical_regions[0].source_region_ids,
            vec!["r1".to_string(), "r2".to_string()]
        );
        assert_eq!(
            plan_reversed.page_plans[0].canonical_regions[0].source_region_ids,
            vec!["r1".to_string(), "r2".to_string()]
        );
        assert_eq!(
            plan_forward.page_plans[0].canonical_regions[0].applied_pdf_point_rect,
            plan_reversed.page_plans[0].canonical_regions[0].applied_pdf_point_rect
        );

        // Assert plan digests are non-empty 64-char hex SHA-256 and identical
        assert_eq!(plan_forward.plan_digest.len(), 64);
        assert_eq!(plan_forward.plan_digest, plan_reversed.plan_digest);
    }

    #[test]
    fn test_planner_safety_margin_and_clamping_warnings() {
        let r_edge = sample_valid_input("r_edge", 0.0, 0.0, 0.1, 0.1);
        let mut input = sample_planner_input_2pages();
        input.region_inputs = vec![r_edge];
        input.policy.safety_margin_points = 10.0;

        let plan = build_secure_redaction_plan(&input).expect("Plan with safety margin clamping");
        assert!(plan
            .warnings
            .iter()
            .any(|w| w.code == "REDACTION_REGION_SAFETY_MARGIN_CLAMPED"));
    }

    #[test]
    fn test_planner_digest_sensitivity() {
        let input_base = sample_planner_input_2pages();
        let plan_base = build_secure_redaction_plan(&input_base).expect("Plan base");
        let digest_base = plan_base.plan_digest.clone();
        assert_eq!(digest_base.len(), 64);

        // Region coordinate change
        let mut input_mod_region = input_base.clone();
        input_mod_region.region_inputs[0].normalized_top_left_rect.x = 0.12;
        let plan_mod_region =
            build_secure_redaction_plan(&input_mod_region).expect("Plan mod region");
        let digest_mod_region = plan_mod_region.plan_digest.clone();
        assert_ne!(digest_base, digest_mod_region);

        // DPI change
        let mut input_mod_dpi = input_base.clone();
        input_mod_dpi.policy.raster_dpi = 400;
        let plan_mod_dpi = build_secure_redaction_plan(&input_mod_dpi).expect("Plan mod dpi");
        let digest_mod_dpi = plan_mod_dpi.plan_digest.clone();
        assert_ne!(digest_base, digest_mod_dpi);
    }

    #[test]
    fn test_planner_resource_limit_blockers() {
        let mut input = sample_planner_input_2pages();
        input.policy.safety_margin_points = 0.0;
        input.region_inputs = (0..MAX_SOURCE_REGION_IDS + 1)
            .map(|i| sample_valid_input(&format!("r{i}"), 0.1, 0.1, 0.2, 0.2))
            .collect();

        let plan = build_secure_redaction_plan(&input).expect("Plan with resource limit");
        assert!(!plan.executable);
        assert!(!plan.ready_for_execution);
        assert!(plan
            .blockers
            .iter()
            .any(|b| b.code == "REDACTION_RESOURCE_LIMIT_EXCEEDED"));
    }

    #[test]
    fn test_planner_produces_no_disk_files() {
        let input = sample_planner_input_2pages();
        let plan = build_secure_redaction_plan(&input);
        assert!(plan.is_ok());
    }

    #[test]
    fn test_unaffected_page_strategy_and_real_digest() {
        let input = sample_planner_input_2pages();
        let plan = build_secure_redaction_plan(&input).expect("Plan should succeed");

        assert_eq!(plan.plan_version, "secure_redaction_plan_v2");
        assert_eq!(plan.plan_digest.len(), 64);
        assert_eq!(
            plan.page_plans[0].strategy,
            PageRedactionStrategy::RasterizePage
        );
        assert_eq!(
            plan.page_plans[1].strategy,
            PageRedactionStrategy::ImportUnaffectedPage
        );
        assert!(!plan.page_plans[1].feature_losses.text_selection_lost);
    }

    #[test]
    fn test_geometry_conversion_parity_with_js() {
        let geom = sample_valid_geometry();
        let norm = NormalizedTopLeftRect {
            x: 0.1,
            y: 0.1,
            width: 0.3,
            height: 0.2,
        };
        let pdf_rect = normalized_top_left_to_pdf_point_rect(&norm, &geom).unwrap();
        assert!((pdf_rect.left - 61.2).abs() < 1e-5);
        assert!((pdf_rect.bottom - 554.4).abs() < 1e-5);
        assert!((pdf_rect.width - 183.6).abs() < 1e-5);
        assert!((pdf_rect.height - 158.4).abs() < 1e-5);
    }

    #[test]
    fn test_aggregated_warning_deduplication() {
        let mut warnings = Vec::new();
        push_aggregated_warning(
            &mut warnings,
            RedactionWarning {
                schema_version: "1.0.0".to_string(),
                code: "REDACTION_REGION_SAFETY_MARGIN_CLAMPED".to_string(),
                message: "Warning 1".to_string(),
                details: Some("Detail 1".to_string()),
            },
        );
        push_aggregated_warning(
            &mut warnings,
            RedactionWarning {
                schema_version: "1.0.0".to_string(),
                code: "REDACTION_REGION_SAFETY_MARGIN_CLAMPED".to_string(),
                message: "Warning 2".to_string(),
                details: Some("Detail 2".to_string()),
            },
        );

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "REDACTION_REGION_SAFETY_MARGIN_CLAMPED");
        assert_eq!(warnings[0].details.as_deref(), Some("Detail 1; Detail 2"));
    }

    #[test]
    fn test_real_no_io_planner_isolation() {
        let temp_dir = std::env::temp_dir();
        let before_entries: Vec<_> = std::fs::read_dir(&temp_dir)
            .expect("Read temp dir before planner")
            .filter_map(|e| e.ok().map(|entry| entry.file_name()))
            .collect();

        let input = sample_planner_input_2pages();
        let plan = build_secure_redaction_plan(&input).expect("Build plan");
        assert!(plan.executable);

        let after_entries: Vec<_> = std::fs::read_dir(&temp_dir)
            .expect("Read temp dir after planner")
            .filter_map(|e| e.ok().map(|entry| entry.file_name()))
            .collect();

        assert_eq!(
            before_entries, after_entries,
            "Planner must produce zero I/O side-effects or temporary files"
        );

        let source = include_str!("secure_redaction.rs");
        let non_test_source = source.split("mod tests").next().unwrap_or(source);
        assert!(!non_test_source.contains("File::create"));
        assert!(!non_test_source.contains("OpenOptions"));
    }

    #[test]
    fn test_semantically_fake_freshly_digested_plans_rejected() {
        let valid_plan = canonical_wire_fixture_plan();
        assert!(validate_secure_redaction_plan(&valid_plan).is_ok());

        // 1. Mutate text_selection_lost when text is present
        let mut plan1 = valid_plan.clone();
        plan1.page_plans[0].feature_losses.text_selection_lost = false;
        plan1.plan_digest = compute_plan_digest(
            &plan1.input_digest,
            plan1.content_generation,
            &plan1.policy,
            &plan1.acknowledgements,
            &plan1.acknowledgement_requirements,
            &plan1.facts,
            &plan1.page_plans,
            &plan1.canonical_regions,
            &plan1.losses,
            &plan1.blockers,
            &plan1.warnings,
            plan1.executable,
            plan1.acknowledgements_satisfied,
            plan1.ready_for_execution,
        );
        assert!(validate_secure_redaction_plan(&plan1).is_err());

        // 2. Mismatch document losses vs page feature losses
        let mut plan2 = valid_plan.clone();
        plan2.losses.searchability_lost = false;
        plan2.plan_digest = compute_plan_digest(
            &plan2.input_digest,
            plan2.content_generation,
            &plan2.policy,
            &plan2.acknowledgements,
            &plan2.acknowledgement_requirements,
            &plan2.facts,
            &plan2.page_plans,
            &plan2.canonical_regions,
            &plan2.losses,
            &plan2.blockers,
            &plan2.warnings,
            plan2.executable,
            plan2.acknowledgements_satisfied,
            plan2.ready_for_execution,
        );
        assert!(validate_secure_redaction_plan(&plan2).is_err());

        // 3. Mismatch acknowledgement requirements
        let mut plan3 = valid_plan.clone();
        plan3.acknowledgement_requirements.text_search_loss_required = false;
        plan3.plan_digest = compute_plan_digest(
            &plan3.input_digest,
            plan3.content_generation,
            &plan3.policy,
            &plan3.acknowledgements,
            &plan3.acknowledgement_requirements,
            &plan3.facts,
            &plan3.page_plans,
            &plan3.canonical_regions,
            &plan3.losses,
            &plan3.blockers,
            &plan3.warnings,
            plan3.executable,
            plan3.acknowledgements_satisfied,
            plan3.ready_for_execution,
        );
        assert!(validate_secure_redaction_plan(&plan3).is_err());

        // 4. Mismatch executable = false without blocker
        let mut plan4 = valid_plan.clone();
        plan4.executable = false;
        plan4.plan_digest = compute_plan_digest(
            &plan4.input_digest,
            plan4.content_generation,
            &plan4.policy,
            &plan4.acknowledgements,
            &plan4.acknowledgement_requirements,
            &plan4.facts,
            &plan4.page_plans,
            &plan4.canonical_regions,
            &plan4.losses,
            &plan4.blockers,
            &plan4.warnings,
            plan4.executable,
            plan4.acknowledgements_satisfied,
            plan4.ready_for_execution,
        );
        assert!(validate_secure_redaction_plan(&plan4).is_err());

        // 5. Unsorted source_region_ids
        let mut plan5 = valid_plan.clone();
        plan5.canonical_regions[0].source_region_ids = vec!["z".to_string(), "a".to_string()];
        plan5.plan_digest = compute_plan_digest(
            &plan5.input_digest,
            plan5.content_generation,
            &plan5.policy,
            &plan5.acknowledgements,
            &plan5.acknowledgement_requirements,
            &plan5.facts,
            &plan5.page_plans,
            &plan5.canonical_regions,
            &plan5.losses,
            &plan5.blockers,
            &plan5.warnings,
            plan5.executable,
            plan5.acknowledgements_satisfied,
            plan5.ready_for_execution,
        );
        assert!(validate_secure_redaction_plan(&plan5).is_err());
    }

    #[test]
    fn test_fresh_digest_duplicate_global_canonical_region_id_rejected() {
        let mut plan = canonical_wire_fixture_plan();
        let mut duplicate_a = plan.page_plans[0].canonical_regions[0].clone();
        duplicate_a.region_id = "canon_duplicate".to_string();
        duplicate_a.source_region_ids = vec!["s_duplicate_a".to_string()];
        let mut duplicate_b = duplicate_a.clone();
        duplicate_b.source_region_ids = vec!["s_duplicate_b".to_string()];

        plan.page_plans[0].canonical_regions = vec![duplicate_a.clone(), duplicate_b.clone()];
        plan.canonical_regions = vec![duplicate_a, duplicate_b];
        plan.plan_digest = compute_plan_digest(
            &plan.input_digest,
            plan.content_generation,
            &plan.policy,
            &plan.acknowledgements,
            &plan.acknowledgement_requirements,
            &plan.facts,
            &plan.page_plans,
            &plan.canonical_regions,
            &plan.losses,
            &plan.blockers,
            &plan.warnings,
            plan.executable,
            plan.acknowledgements_satisfied,
            plan.ready_for_execution,
        );

        let error = validate_secure_redaction_plan(&plan).unwrap_err();
        if let crate::document_session::SessionJobOperationError::Failed(failure) = error {
            assert_eq!(failure.code, "REDACTION_DUPLICATE_REGION_ID");
            assert_eq!(
                failure.message,
                "Duplicate global canonical region ID found: 'canon_duplicate'"
            );
        } else {
            panic!("Expected duplicate global canonical region ID failure");
        }
    }

    #[test]
    fn test_adversarial_planner_self_validation_with_unknown_facts() {
        let mut input = sample_planner_input_2pages();
        input.page_facts[0].complex_resources = InspectionFactState::Unknown;
        let plan = build_secure_redaction_plan(&input).expect("Planner handles unknown facts");

        assert!(!plan.executable);
        assert!(!plan.ready_for_execution);
        assert!(plan
            .blockers
            .iter()
            .any(|b| b.code == "REDACTION_BLOCKED_DOCUMENT_FEATURE"));
        assert!(validate_secure_redaction_plan(&plan).is_ok());
    }

    #[test]
    fn test_adversarial_page_blocker_tampering_rejected_with_fresh_digest() {
        let mut input = sample_planner_input_2pages();
        input.page_facts[0].complex_resources = InspectionFactState::Unknown;
        let mut plan = build_secure_redaction_plan(&input).expect("Plan with page blocker");

        // Tamper: remove page blocker and mark page executable
        plan.page_plans[0].blockers.clear();
        plan.page_plans[0].executable = true;

        // Recompute fresh digest for tampered plan
        plan.plan_digest = compute_plan_digest(
            &plan.input_digest,
            plan.content_generation,
            &plan.policy,
            &plan.acknowledgements,
            &plan.acknowledgement_requirements,
            &plan.facts,
            &plan.page_plans,
            &plan.canonical_regions,
            &plan.losses,
            &plan.blockers,
            &plan.warnings,
            plan.executable,
            plan.acknowledgements_satisfied,
            plan.ready_for_execution,
        );

        assert!(validate_secure_redaction_plan(&plan).is_err());
    }

    #[test]
    fn test_adversarial_missing_top_level_blocker_rejected_with_fresh_digest() {
        let mut input = sample_planner_input_2pages();
        input.page_facts[0].complex_resources = InspectionFactState::Unknown;
        let mut plan = build_secure_redaction_plan(&input).expect("Plan with page blocker");

        // Tamper: remove top-level blockers
        plan.blockers.clear();

        // Recompute fresh digest
        plan.plan_digest = compute_plan_digest(
            &plan.input_digest,
            plan.content_generation,
            &plan.policy,
            &plan.acknowledgements,
            &plan.acknowledgement_requirements,
            &plan.facts,
            &plan.page_plans,
            &plan.canonical_regions,
            &plan.losses,
            &plan.blockers,
            &plan.warnings,
            plan.executable,
            plan.acknowledgements_satisfied,
            plan.ready_for_execution,
        );

        assert!(validate_secure_redaction_plan(&plan).is_err());
    }

    #[test]
    fn test_adversarial_altered_blocker_details_rejected_with_fresh_digest() {
        let mut input = sample_planner_input_2pages();
        input.page_facts[0].complex_resources = InspectionFactState::Unknown;
        let mut plan = build_secure_redaction_plan(&input).expect("Plan with page blocker");

        // Tamper: alter blocker details
        plan.blockers[0].details = Some("Tampered detail text".to_string());

        // Recompute fresh digest
        plan.plan_digest = compute_plan_digest(
            &plan.input_digest,
            plan.content_generation,
            &plan.policy,
            &plan.acknowledgements,
            &plan.acknowledgement_requirements,
            &plan.facts,
            &plan.page_plans,
            &plan.canonical_regions,
            &plan.losses,
            &plan.blockers,
            &plan.warnings,
            plan.executable,
            plan.acknowledgements_satisfied,
            plan.ready_for_execution,
        );

        assert!(validate_secure_redaction_plan(&plan).is_err());
    }

    #[test]
    fn test_adversarial_encryption_unknown_self_validation() {
        let mut input = sample_planner_input_2pages();
        input.document_facts.encryption = InspectionFactState::Unknown;
        let plan =
            build_secure_redaction_plan(&input).expect("Planner handles unknown encryption state");

        assert!(!plan.executable);
        assert!(!plan.ready_for_execution);
        assert!(plan
            .blockers
            .iter()
            .any(|b| b.code == "REDACTION_ENCRYPTED_INPUT_UNSUPPORTED"));
        assert!(validate_secure_redaction_plan(&plan).is_ok());
    }

    #[test]
    fn test_adversarial_signature_unknown_self_validation() {
        let mut input = sample_planner_input_2pages();
        input.document_facts.signature_dictionary = InspectionFactState::Unknown;
        let plan =
            build_secure_redaction_plan(&input).expect("Planner handles unknown signature state");

        assert!(!plan.executable);
        assert!(!plan.ready_for_execution);
        assert!(plan
            .blockers
            .iter()
            .any(|b| b.code == "REDACTION_SIGNED_INPUT_UNSUPPORTED"));
        assert!(validate_secure_redaction_plan(&plan).is_ok());
    }

    #[test]
    fn test_adversarial_altered_blocker_message_rejected_with_fresh_digest() {
        let mut input = sample_planner_input_2pages();
        input.page_facts[0].complex_resources = InspectionFactState::Unknown;
        let mut plan = build_secure_redaction_plan(&input).expect("Plan with page blocker");

        // Tamper: alter blocker message
        plan.blockers[0].message = "Arbitrary altered blocker message".to_string();

        // Recompute fresh digest
        plan.plan_digest = compute_plan_digest(
            &plan.input_digest,
            plan.content_generation,
            &plan.policy,
            &plan.acknowledgements,
            &plan.acknowledgement_requirements,
            &plan.facts,
            &plan.page_plans,
            &plan.canonical_regions,
            &plan.losses,
            &plan.blockers,
            &plan.warnings,
            plan.executable,
            plan.acknowledgements_satisfied,
            plan.ready_for_execution,
        );

        assert!(validate_secure_redaction_plan(&plan).is_err());
    }

    #[test]
    fn test_source_id_exact_boundary_100() {
        let mut input = sample_planner_input_2pages();
        input.policy.safety_margin_points = 0.0;
        input.region_inputs = (0..MAX_SOURCE_REGION_IDS)
            .map(|i| sample_valid_input(&format!("r{i}"), 0.1, 0.1, 0.2, 0.2))
            .collect();

        let plan = build_secure_redaction_plan(&input).expect("100 source IDs plan");
        assert!(plan.executable);
        assert!(plan.ready_for_execution);
        assert_eq!(plan.page_plans[0].canonical_regions.len(), 1);
        assert_eq!(
            plan.page_plans[0].canonical_regions[0]
                .source_region_ids
                .len(),
            100
        );
        assert!(validate_secure_redaction_plan(&plan).is_ok());
    }

    #[test]
    fn test_source_id_plus_one_overflow_101() {
        let mut input = sample_planner_input_2pages();
        input.policy.safety_margin_points = 0.0;
        input.region_inputs = (0..MAX_SOURCE_REGION_IDS + 1)
            .map(|i| sample_valid_input(&format!("r{i}"), 0.1, 0.1, 0.2, 0.2))
            .collect();

        let plan = build_secure_redaction_plan(&input).expect("101 source IDs plan");
        assert!(!plan.executable);
        assert!(!plan.ready_for_execution);
        assert!(plan
            .blockers
            .iter()
            .any(|b| b.code == "REDACTION_RESOURCE_LIMIT_EXCEEDED"));
        assert!(validate_secure_redaction_plan(&plan).is_ok());

        // Evidence of coverage preservation
        let total_source_ids: usize = plan.page_plans[0]
            .canonical_regions
            .iter()
            .map(|r| r.source_region_ids.len())
            .sum();
        assert_eq!(
            total_source_ids, 101,
            "All 101 source IDs must be preserved in canonical regions without silent truncation"
        );
    }

    #[test]
    fn test_external_oversized_wire_rejected_even_with_resource_blocker() {
        let mut plan = canonical_wire_fixture_plan();
        let oversized_regions: Vec<_> = (0..MAX_CANONICAL_REGIONS_PER_PAGE + 1)
            .map(|i| {
                let mut r = plan.page_plans[0].canonical_regions[0].clone();
                r.region_id = format!("r{i}");
                r.source_region_ids = vec![format!("src_{i}")];
                r
            })
            .collect();

        plan.page_plans[0].canonical_regions = oversized_regions;
        plan.page_plans[0].blockers.push(RedactionBlocker {
            schema_version: "1.0.0".to_string(),
            code: "REDACTION_RESOURCE_LIMIT_EXCEEDED".to_string(),
            message: "fake resource blocker".to_string(),
            details: None,
        });
        plan.page_plans[0].executable = false;

        assert!(validate_page_redaction_plan(&plan.page_plans[0], plan.facts.total_pages).is_err());
    }

    #[test]
    fn test_adversarial_strict_manifest_parity_key_sets() {
        let contract_str =
            include_str!("../tests/fixtures/contracts/secure-redaction-contract-v1.json");
        let manifest_str = include_str!("../tests/fixtures/redaction/manifest.json");

        let contract: serde_json::Value = serde_json::from_str(contract_str).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(manifest_str).unwrap();

        let manifest_limits: HashSet<_> = manifest["limits"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let expected_manifest_limits: HashSet<_> = vec![
            "maxPixelsPerAffectedPage".to_string(),
            "maxTotalEstimatedAffectedPixels".to_string(),
            "maxReportSizeBytes".to_string(),
            "maxSourceRegionIdsPerCanonicalRegion".to_string(),
            "maxCanonicalRegionsPerPage".to_string(),
        ]
        .into_iter()
        .collect();
        assert_eq!(manifest_limits, expected_manifest_limits);

        let manifest_merge: HashSet<_> = manifest["merge_profile"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let expected_manifest_merge: HashSet<_> = vec![
            "adjacencyTolerancePoints".to_string(),
            "mergeEdgeTouching".to_string(),
            "mergeCornerTouching".to_string(),
            "geometryComparisonEpsilon".to_string(),
        ]
        .into_iter()
        .collect();
        assert_eq!(manifest_merge, expected_manifest_merge);

        // Shared duplicate limits parity check
        let contract_max_source = contract["limits"]["MAX_SOURCE_REGION_IDS"]
            .as_u64()
            .unwrap();
        let manifest_max_source = manifest["limits"]["maxSourceRegionIdsPerCanonicalRegion"]
            .as_u64()
            .unwrap();
        assert_eq!(contract_max_source, manifest_max_source);

        let contract_max_canonical = contract["limits"]["MAX_CANONICAL_REGIONS_PER_PAGE"]
            .as_u64()
            .unwrap();
        let manifest_max_canonical = manifest["limits"]["maxCanonicalRegionsPerPage"]
            .as_u64()
            .unwrap();
        assert_eq!(contract_max_canonical, manifest_max_canonical);
    }

    #[test]
    fn test_canonical_region_ordering_validation_wire_rejection() {
        let mut page_plan = sample_valid_page_plan();
        let mut reg1 = page_plan.canonical_regions[0].clone();
        let mut reg2 = page_plan.canonical_regions[0].clone();

        reg1.region_id = "\u{E000}".to_string();
        reg1.source_region_ids = vec!["s1".to_string()];
        reg2.region_id = "😀".to_string();
        reg2.source_region_ids = vec!["s2".to_string()];

        page_plan.canonical_regions = vec![reg1.clone(), reg2.clone()];
        assert!(validate_page_redaction_plan(&page_plan, 1).is_ok());

        page_plan.canonical_regions = vec![reg2, reg1];
        let error = validate_page_redaction_plan(&page_plan, 1).unwrap_err();
        if let crate::document_session::SessionJobOperationError::Failed(failure) = error {
            assert_eq!(failure.code, "REDACTION_INVALID_POLICY");
            assert_eq!(
                failure.message,
                "Canonical regions must be in canonical sort order (pageIndex, bottom, left, regionId)"
            );
        } else {
            panic!("Expected canonical ordering policy failure");
        }
    }

    #[test]
    fn test_unicode_region_id_tie_break_utf8_order() {
        let mut reg1 = sample_valid_page_plan().canonical_regions[0].clone();
        let mut reg2 = reg1.clone();

        reg1.region_id = "\u{E000}".to_string();
        reg2.region_id = "😀".to_string();

        assert_eq!(
            compare_canonical_redaction_regions(&reg1, &reg2),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_near_equal_numeric_bottom_exact_ordering() {
        let mut reg_a = sample_valid_page_plan().canonical_regions[0].clone();
        reg_a.region_id = "z".to_string();
        reg_a.applied_pdf_point_rect.bottom = 100.0;
        reg_a.applied_pdf_point_rect.left = 500.0;

        let mut reg_b = reg_a.clone();
        reg_b.region_id = "a".to_string();
        reg_b.applied_pdf_point_rect.bottom = 100.0000005;
        reg_b.applied_pdf_point_rect.left = 0.0;

        assert_eq!(
            compare_canonical_redaction_regions(&reg_a, &reg_b),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_details_part_sorting_exact_unicode_vector() {
        let mut warnings = Vec::new();
        push_aggregated_warning(
            &mut warnings,
            RedactionWarning {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_WARNING_TEST".to_string(),
                message: "Test message".to_string(),
                details: Some("😀".to_string()),
            },
        );
        push_aggregated_warning(
            &mut warnings,
            RedactionWarning {
                schema_version: REDACTION_SCHEMA_VERSION.to_string(),
                code: "REDACTION_WARNING_TEST".to_string(),
                message: "Test message".to_string(),
                details: Some("\u{E000}".to_string()),
            },
        );

        assert_eq!(warnings[0].details.as_deref(), Some("\u{E000}; 😀"));
    }
}
