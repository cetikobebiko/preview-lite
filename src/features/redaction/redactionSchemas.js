/**
 * @fileoverview Secure Redaction Domain Contracts & Validation Schemas
 * Sürümlü Redaksiyon Region, Policy ve Sonuç Sözleşmeleri (Prompt 8B-2.3 Integrity & Numeric Parity)
 */

export const REDACTION_SCHEMA_VERSION = '1.0.0';
// v2 includes Trim/Bleed/Art boxes in the signed canonical geometry. Keep v1
// distinct rather than widening its wire contract in place.
export const SECURE_REDACTION_PLAN_VERSION = 'secure_redaction_plan_v2';
export const SECURE_REDACTION_VERIFICATION_VERSION = 'secure_redaction_verifier_v1';
export const REQUIRED_SECURE_REDACTION_VERIFICATION_CHECKS = Object.freeze([
  'SOURCE_PLAN_BINDING', 'TRANSFORM_REPORT_CONSISTENCY', 'ARTIFACT_PATH_INTEGRITY',
  'ARTIFACT_DIGEST_AND_SIZE', 'DUAL_PARSER_REOPEN', 'FULL_REWRITE_STRUCTURE',
  'FORBIDDEN_DOCUMENT_STRUCTURES_ABSENT', 'PAGE_COUNT_AND_STRATEGY', 'PAGE_GEOMETRY',
  'AFFECTED_PAGE_IMAGE_ONLY', 'AFFECTED_PAGE_INTERACTIVE_CONTENT_ABSENT',
  'AFFECTED_PAGE_REDACTION_PIXEL_COVERAGE', 'UNAFFECTED_PAGE_EQUIVALENCE',
  'ARTIFACT_FINAL_DIGEST_STABILITY',
]);

export const U32_MIN = 0;
export const U32_MAX = 4_294_967_295;
export const I32_MIN = -2_147_483_648;
export const I32_MAX = 2_147_483_647;

export const REDACTION_LIMITS = Object.freeze({
  MAX_REGIONS_PER_INPUT: 1000,
  MAX_REGION_ID_LEN: 128,
  MAX_SOURCE_LEN: 256,
  MAX_SOURCE_REGION_IDS: 100,
  MAX_CANONICAL_REGIONS_PER_PAGE: 1000,
  MAX_PAGE_PLANS: 1000,
  MAX_PIXELS_PER_PAGE: 100_000_000,
  MAX_TOTAL_ESTIMATED_PIXELS: 1_000_000_000,
  MAX_REPORT_SIZE_BYTES: 10_485_760,
  // Matches Rust's internal secure-redaction artifact resource policy. The
  // Rust wire value remains u64, while this explicit cap stays exactly
  // representable by JavaScript numbers.
  MAX_SECURE_REDACTION_OUTPUT_BYTES: 1_073_741_824,
  MAX_RASTER_DIMENSION: 16_384,
  ADJACENCY_TOLERANCE_POINTS: 0.0,
  MERGE_EDGE_TOUCHING: true,
  MERGE_CORNER_TOUCHING: false,
  MAX_BLOCKERS: 100,
  MAX_WARNINGS: 500,
  MAX_VERIFICATION_CHECKS: 100,
  MAX_BLOCKER_CODE_LEN: 128,
  MAX_WARNING_CODE_LEN: 128,
  MAX_USER_SAFE_MESSAGE_LEN: 2048,
  MAX_DETAILS_LEN: 4096,
  MAX_IDENTITY_LEN: 128,
  MAX_VERIFICATION_CHECK_CODE_LEN: 256,
  MAX_VERIFICATION_CHECK_MESSAGE_LEN: 2048,
  MAX_VERIFICATION_CHECK_DETAILS_LEN: 4096,
  MAX_PATH_LEN: 4096,
  MIN_RASTER_DPI: 72,
  MAX_RASTER_DPI: 600,
  MIN_SAFETY_MARGIN_POINTS: 0.0,
  MAX_SAFETY_MARGIN_POINTS: 36.0,
});

export const CANONICAL_REDACTION_LIMITS = Object.freeze({
  maxPixelsPerAffectedPage: 100_000_000,
  maxTotalEstimatedAffectedPixels: 1_000_000_000,
  maxReportSizeBytes: 10_485_760,
  maxSourceRegionIdsPerCanonicalRegion: 100,
  maxCanonicalRegionsPerPage: 1000,
});

export const CANONICAL_MERGE_PROFILE = Object.freeze({
  adjacencyTolerancePoints: 0.0,
  mergeEdgeTouching: true,
  mergeCornerTouching: false,
  geometryComparisonEpsilon: 1e-6,
});

export const REDACTION_ERROR_CODES = Object.freeze({
  REDACTION_INVALID_POLICY: 'REDACTION_INVALID_POLICY',
  REDACTION_POLICY_VERSION_UNSUPPORTED: 'REDACTION_POLICY_VERSION_UNSUPPORTED',
  REDACTION_INVALID_REGION: 'REDACTION_INVALID_REGION',
  REDACTION_REGION_OUT_OF_BOUNDS: 'REDACTION_REGION_OUT_OF_BOUNDS',
  REDACTION_DUPLICATE_REGION_ID: 'REDACTION_DUPLICATE_REGION_ID',
  REDACTION_REGION_LIMIT_EXCEEDED: 'REDACTION_REGION_LIMIT_EXCEEDED',
  REDACTION_PAGE_NOT_FOUND: 'REDACTION_PAGE_NOT_FOUND',
  REDACTION_PAGE_GEOMETRY_MISMATCH: 'REDACTION_PAGE_GEOMETRY_MISMATCH',
  REDACTION_PLAN_STALE: 'REDACTION_PLAN_STALE',
  REDACTION_PLAN_DIGEST_MISMATCH: 'REDACTION_PLAN_DIGEST_MISMATCH',
  REDACTION_BLOCKED_DOCUMENT_FEATURE: 'REDACTION_BLOCKED_DOCUMENT_FEATURE',
  REDACTION_ENCRYPTED_INPUT_UNSUPPORTED: 'REDACTION_ENCRYPTED_INPUT_UNSUPPORTED',
  REDACTION_SIGNED_INPUT_UNSUPPORTED: 'REDACTION_SIGNED_INPUT_UNSUPPORTED',
  REDACTION_XFA_INPUT_UNSUPPORTED: 'REDACTION_XFA_INPUT_UNSUPPORTED',
  REDACTION_RESOURCE_LIMIT_EXCEEDED: 'REDACTION_RESOURCE_LIMIT_EXCEEDED',
});

export class RedactionError extends Error {
  /**
   * @param {string} code
   * @param {string} message
   * @param {string} [details]
   */
  constructor(code, message, details) {
    super(message);
    this.name = 'RedactionError';
    this.code = code;
    this.details = details;
  }
}

/**
 * Canonical UTF-8 unsigned-byte lexicographical ascending comparator
 * @param {string} a
 * @param {string} b
 * @returns {number} -1 if a < b, 1 if a > b, 0 if equal
 */
export function compareCanonicalUtf8Strings(a, b) {
  if (a === b) return 0;
  const encoder = new TextEncoder();
  const aBytes = encoder.encode(a);
  const bBytes = encoder.encode(b);
  const minLen = Math.min(aBytes.length, bBytes.length);
  for (let i = 0; i < minLen; i++) {
    if (aBytes[i] !== bBytes[i]) {
      return aBytes[i] < bBytes[i] ? -1 : 1;
    }
  }
  if (aBytes.length !== bBytes.length) {
    return aBytes.length < bBytes.length ? -1 : 1;
  }
  return 0;
}

/**
 * Canonical region comparator matching exact specification:
 * 1. pageIndex exact integer
 * 2. appliedPdfPointRect.bottom exact finite number (no epsilon)
 * 3. appliedPdfPointRect.left exact finite number (no epsilon)
 * 4. regionId canonical UTF-8 comparator
 * @param {Object} a
 * @param {Object} b
 * @returns {number} -1, 0, or 1
 */
export function compareCanonicalRegions(a, b) {
  if (a.pageIndex !== b.pageIndex) {
    return a.pageIndex < b.pageIndex ? -1 : 1;
  }
  if (a.appliedPdfPointRect.bottom !== b.appliedPdfPointRect.bottom) {
    return a.appliedPdfPointRect.bottom < b.appliedPdfPointRect.bottom ? -1 : 1;
  }
  if (a.appliedPdfPointRect.left !== b.appliedPdfPointRect.left) {
    return a.appliedPdfPointRect.left < b.appliedPdfPointRect.left ? -1 : 1;
  }
  return compareCanonicalUtf8Strings(a.regionId, b.regionId);
}


/**
 * Strict unknown field rejection helper
 * @param {Record<string, any>} obj
 * @param {string[]} allowedKeys
 * @param {string} objectName
 */
function assertNoUnknownFields(obj, allowedKeys, objectName) {
  if (!obj || typeof obj !== 'object' || Array.isArray(obj)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `${objectName} must be a valid object`
    );
  }
  const allowedSet = new Set(allowedKeys);
  for (const key of Object.keys(obj)) {
    if (!allowedSet.has(key)) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        `Unknown field '${key}' is rejected in ${objectName}`
      );
    }
  }
}

/**
 * Exclusive alias reader (fails closed if both camelCase and snake_case exist as own properties)
 * @param {Record<string, any>} raw
 * @param {string} camelKey
 * @param {string} snakeKey
 * @param {boolean} isRequired
 * @param {string} fieldName
 * @param {string} objectName
 */
function readExclusiveAlias(raw, camelKey, snakeKey, isRequired, fieldName, objectName) {
  const hasCamel = Object.prototype.hasOwnProperty.call(raw, camelKey);
  const hasSnake = camelKey !== snakeKey && Object.prototype.hasOwnProperty.call(raw, snakeKey);

  if (hasCamel && hasSnake) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `Ambiguous alias conflict: both '${camelKey}' and '${snakeKey}' provided in ${objectName}`
    );
  }

  if (hasCamel) return raw[camelKey];
  if (hasSnake) return raw[snakeKey];

  if (isRequired) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `Required field '${fieldName}' is missing in ${objectName}`
    );
  }

  return undefined;
}

/**
 * Strict presence & nullability validator for optional object fields.
 * Prevents truthiness coercion of false, 0, "", NaN into null.
 * @template T
 * @param {any} value
 * @param {(val: any) => T} normalizeFn
 * @param {string} fieldName
 * @returns {T|null}
 */
export function normalizeNullableObject(value, normalizeFn, fieldName) {
  if (value === null || value === undefined) {
    return null;
  }
  if (typeof value !== 'object' || Array.isArray(value)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `${fieldName} must be a valid object or null, got ${typeof value}`
    );
  }
  return normalizeFn(value);
}

/**
 * UTF-8 byte length measurement (Parity with Rust str.as_bytes().len())
 * @param {string} str
 * @returns {number}
 */
export function getUtf8ByteLength(str) {
  return new TextEncoder().encode(str).length;
}

/**
 * SHA-256 Digest format validator (64 lowercase hex characters)
 * @param {string} digest
 * @param {string} fieldName
 */
export function validateSha256Digest(digest, fieldName) {
  if (typeof digest !== 'string' || getUtf8ByteLength(digest) !== 64 || !/^[0-9a-f]{64}$/.test(digest)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_PLAN_DIGEST_MISMATCH,
      `${fieldName} must be exactly 64 lower-case hex characters (SHA-256)`
    );
  }
}

export const GEOMETRY_EPSILON = 1e-6;

export async function sha256Hex(bytes) {
  if (typeof globalThis !== 'undefined' && globalThis.crypto && globalThis.crypto.subtle) {
    const hashBuffer = await globalThis.crypto.subtle.digest('SHA-256', bytes);
    const hashArray = Array.from(new Uint8Array(hashBuffer));
    return hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
  }
  throw new Error('sha256 engine unavailable (Web Crypto globalThis.crypto.subtle required)');
}

export class CanonicalSerializer {
  constructor() {
    this.buffer = [];
  }

  writeTag(tag) {
    this.buffer.push(tag & 0xff);
  }

  writeU32BE(val) {
    const u = val >>> 0;
    this.buffer.push((u >>> 24) & 0xff, (u >>> 16) & 0xff, (u >>> 8) & 0xff, u & 0xff);
  }

  writeU64BE(val) {
    const big = typeof val === 'bigint' ? val : BigInt(val);
    const buf = new ArrayBuffer(8);
    new DataView(buf).setBigUint64(0, big, false);
    const bytes = new Uint8Array(buf);
    for (let i = 0; i < 8; i++) this.buffer.push(bytes[i]);
  }

  writeString(str) {
    this.writeTag(0x01);
    const bytes = new TextEncoder().encode(str);
    this.writeU32BE(bytes.length);
    for (let i = 0; i < bytes.length; i++) this.buffer.push(bytes[i]);
  }

  writeBool(b) {
    this.writeTag(0x02);
    this.writeU32BE(1);
    this.buffer.push(b ? 1 : 0);
  }

  writeU32(val) {
    this.writeTag(0x03);
    this.writeU32BE(4);
    this.writeU32BE(val);
  }

  writeI32(val) {
    this.writeTag(0x0a);
    this.writeU32BE(4);
    const buf = new ArrayBuffer(4);
    new DataView(buf).setInt32(0, val, false);
    const bytes = new Uint8Array(buf);
    for (let i = 0; i < 4; i++) this.buffer.push(bytes[i]);
  }

  writeU64(val) {
    this.writeTag(0x04);
    this.writeU32BE(8);
    this.writeU64BE(val);
  }

  writeF64(val) {
    this.writeTag(0x05);
    this.writeU32BE(8);
    const norm = val === 0 ? 0 : val;
    const buf = new ArrayBuffer(8);
    new DataView(buf).setFloat64(0, norm, false);
    const bytes = new Uint8Array(buf);
    for (let i = 0; i < 8; i++) this.buffer.push(bytes[i]);
  }

  writeNull() {
    this.writeTag(0x06);
    this.writeU32BE(0);
  }

  writeField(name, writeValFn) {
    this.writeTag(0x09);
    const lenPos = this.buffer.length;
    this.writeU32BE(0);
    const startPos = this.buffer.length;

    this.writeString(name);
    writeValFn(this);

    const fieldLen = this.buffer.length - startPos;
    const u = fieldLen >>> 0;
    this.buffer[lenPos] = (u >>> 24) & 0xff;
    this.buffer[lenPos + 1] = (u >>> 16) & 0xff;
    this.buffer[lenPos + 2] = (u >>> 8) & 0xff;
    this.buffer[lenPos + 3] = u & 0xff;
  }

  writeListStart(count) {
    this.writeTag(0x07);
    this.writeU32BE(count);
  }

  writeObjectStart(fieldCount) {
    this.writeTag(0x08);
    this.writeU32BE(fieldCount);
  }

  getBytes() {
    return new Uint8Array(this.buffer);
  }

  async finalizeDigest() {
    return await sha256Hex(this.getBytes());
  }
}

export function writeCanonicalPageGeometryFields(s, geom) {
  s.writeObjectStart(10);
  s.writeField('schema_version', s => s.writeString(REDACTION_SCHEMA_VERSION));
  s.writeField('page_index', s => s.writeU32(geom.pageIndex));
  s.writeField('media_box', s => {
    s.writeObjectStart(4);
    s.writeField('left', b => b.writeF64(geom.mediaBox.left));
    s.writeField('bottom', b => b.writeF64(geom.mediaBox.bottom));
    s.writeField('width', b => b.writeF64(geom.mediaBox.width));
    s.writeField('height', b => b.writeF64(geom.mediaBox.height));
  });
  s.writeField('crop_box', s => {
    if (geom.cropBox) {
      s.writeObjectStart(4);
      s.writeField('left', b => b.writeF64(geom.cropBox.left));
      s.writeField('bottom', b => b.writeF64(geom.cropBox.bottom));
      s.writeField('width', b => b.writeF64(geom.cropBox.width));
      s.writeField('height', b => b.writeF64(geom.cropBox.height));
    } else {
      s.writeNull();
    }
  });
  for (const [field, box] of [['trim_box', geom.trimBox], ['bleed_box', geom.bleedBox], ['art_box', geom.artBox]]) {
    s.writeField(field, s => {
      if (box) {
        s.writeObjectStart(4);
        s.writeField('left', b => b.writeF64(box.left));
        s.writeField('bottom', b => b.writeF64(box.bottom));
        s.writeField('width', b => b.writeF64(box.width));
        s.writeField('height', b => b.writeF64(box.height));
      } else {
        s.writeNull();
      }
    });
  }
  s.writeField('effective_box', s => {
    s.writeObjectStart(4);
    s.writeField('left', b => b.writeF64(geom.effectiveBox.left));
    s.writeField('bottom', b => b.writeF64(geom.effectiveBox.bottom));
    s.writeField('width', b => b.writeF64(geom.effectiveBox.width));
    s.writeField('height', b => b.writeF64(geom.effectiveBox.height));
  });
  s.writeField('rotation', s => s.writeU32(geom.rotation));
  s.writeField('geometry_profile', s => s.writeString('standard_effective_box_v1'));
}

export async function computeGeometryDigest(geom) {
  const normGeom = normalizeCanonicalPageGeometry(geom);
  const serializer = new CanonicalSerializer();
  writeCanonicalPageGeometryFields(serializer, normGeom);
  return await serializer.finalizeDigest();
}

function serializeCanonicalRegion(s, r) {
  s.writeObjectStart(10);
  s.writeField('region_id', s => s.writeString(r.regionId));
  s.writeField('source_region_ids', s => {
    s.writeListStart(r.sourceRegionIds.length);
    for (const id of r.sourceRegionIds) {
      s.writeString(id);
    }
  });
  s.writeField('page_index', s => s.writeU32(r.pageIndex));
  s.writeField('original_normalized_rect', s => {
    s.writeObjectStart(4);
    s.writeField('x', s => s.writeF64(r.originalNormalizedRect.x));
    s.writeField('y', s => s.writeF64(r.originalNormalizedRect.y));
    s.writeField('width', s => s.writeF64(r.originalNormalizedRect.width));
    s.writeField('height', s => s.writeF64(r.originalNormalizedRect.height));
  });
  s.writeField('base_pdf_point_rect', s => {
    s.writeObjectStart(4);
    s.writeField('left', s => s.writeF64(r.basePdfPointRect.left));
    s.writeField('bottom', s => s.writeF64(r.basePdfPointRect.bottom));
    s.writeField('width', s => s.writeF64(r.basePdfPointRect.width));
    s.writeField('height', s => s.writeF64(r.basePdfPointRect.height));
  });
  s.writeField('requested_safety_margin_points', s => s.writeF64(r.requestedSafetyMarginPoints));
  s.writeField('applied_pdf_point_rect', s => {
    s.writeObjectStart(4);
    s.writeField('left', s => s.writeF64(r.appliedPdfPointRect.left));
    s.writeField('bottom', s => s.writeF64(r.appliedPdfPointRect.bottom));
    s.writeField('width', s => s.writeF64(r.appliedPdfPointRect.width));
    s.writeField('height', s => s.writeF64(r.appliedPdfPointRect.height));
  });
  s.writeField('applied_margins', s => {
    s.writeObjectStart(4);
    s.writeField('left', s => s.writeF64(r.appliedMargins.left));
    s.writeField('right', s => s.writeF64(r.appliedMargins.right));
    s.writeField('top', s => s.writeF64(r.appliedMargins.top));
    s.writeField('bottom', s => s.writeF64(r.appliedMargins.bottom));
  });
  s.writeField('boundary_adjusted', s => s.writeBool(r.boundaryAdjusted));
  s.writeField('device_pixel_rect', s => {
    if (r.devicePixelRect) {
      s.writeObjectStart(4);
      s.writeField('left', s => s.writeI32(r.devicePixelRect.left));
      s.writeField('top', s => s.writeI32(r.devicePixelRect.top));
      s.writeField('width', s => s.writeU32(r.devicePixelRect.width));
      s.writeField('height', s => s.writeU32(r.devicePixelRect.height));
    } else {
      s.writeNull();
    }
  });
}

function serializeBlocker(s, b) {
  s.writeObjectStart(4);
  s.writeField('schema_version', s => s.writeString(b.schemaVersion));
  s.writeField('code', s => s.writeString(b.code));
  s.writeField('message', s => s.writeString(b.message));
  s.writeField('details', s => {
    if (b.details !== null && b.details !== undefined) {
      s.writeString(b.details);
    } else {
      s.writeNull();
    }
  });
}

function serializeWarning(s, w) {
  s.writeObjectStart(4);
  s.writeField('schema_version', s => s.writeString(w.schemaVersion));
  s.writeField('code', s => s.writeString(w.code));
  s.writeField('message', s => s.writeString(w.message));
  s.writeField('details', s => {
    if (w.details !== null && w.details !== undefined) {
      s.writeString(w.details);
    } else {
      s.writeNull();
    }
  });
}

function serializePageRedactionFacts(s, f) {
  s.writeObjectStart(18);
  s.writeField('schema_version', s => s.writeString(f.schemaVersion));
  s.writeField('page_index', s => s.writeU32(f.pageIndex));
  s.writeField('text', s => s.writeString(f.text));
  s.writeField('images', s => s.writeString(f.images));
  s.writeField('vector_paths', s => s.writeString(f.vectorPaths));
  s.writeField('form_xobjects', s => s.writeString(f.formXobjects));
  s.writeField('annotations', s => s.writeString(f.annotations));
  s.writeField('link_annotations', s => s.writeString(f.linkAnnotations));
  s.writeField('widgets', s => s.writeString(f.widgets));
  s.writeField('transparency', s => s.writeString(f.transparency));
  s.writeField('optional_content', s => s.writeString(f.optionalContent));
  s.writeField('complex_resources', s => s.writeString(f.complexResources));
  s.writeField('structure_association', s => s.writeString(f.structureAssociation));
  s.writeField('page_actions', s => s.writeString(f.pageActions));
  s.writeField('spot_colors', s => s.writeString(f.spotColors));
  s.writeField('complex_color_spaces', s => s.writeString(f.complexColorSpaces));
  s.writeField('geometry_profile', s => s.writeString(f.geometryProfile));
  s.writeField('rotation', s => s.writeU32(f.rotation));
}

function serializeDocumentRedactionFacts(s, f) {
  s.writeObjectStart(18);
  s.writeField('schema_version', s => s.writeString(f.schemaVersion));
  s.writeField('total_pages', s => s.writeU32(f.totalPages));
  s.writeField('encryption', s => s.writeString(f.encryption));
  s.writeField('signature_dictionary', s => s.writeString(f.signatureDictionary));
  s.writeField('signature_field', s => s.writeString(f.signatureField));
  s.writeField('byte_range', s => s.writeString(f.byteRange));
  s.writeField('acro_form', s => s.writeString(f.acroForm));
  s.writeField('xfa', s => s.writeString(f.xfa));
  s.writeField('annotations', s => s.writeString(f.annotations));
  s.writeField('optional_content', s => s.writeString(f.optionalContent));
  s.writeField('embedded_files', s => s.writeString(f.embeddedFiles));
  s.writeField('javascript', s => s.writeString(f.javascript));
  s.writeField('incremental_revisions', s => s.writeString(f.incrementalRevisions));
  s.writeField('structure_tree', s => s.writeString(f.structureTree));
  s.writeField('outlines', s => s.writeString(f.outlines));
  s.writeField('names_dictionary', s => s.writeString(f.namesDictionary));
  s.writeField('document_actions', s => s.writeString(f.documentActions));
  s.writeField('complex_document_resources', s => s.writeString(f.complexDocumentResources));
}

function serializeRedactionFeatureLosses(s, l) {
  s.writeObjectStart(13);
  s.writeField('schema_version', s => s.writeString(l.schemaVersion));
  s.writeField('text_selection_lost', s => s.writeBool(l.textSelectionLost));
  s.writeField('searchability_lost', s => s.writeBool(l.searchabilityLost));
  s.writeField('accessibility_tags_lost', s => s.writeBool(l.accessibilityTagsLost));
  s.writeField('links_lost', s => s.writeBool(l.linksLost));
  s.writeField('annotations_lost', s => s.writeBool(l.annotationsLost));
  s.writeField('forms_lost', s => s.writeBool(l.formsLost));
  s.writeField('vector_quality_lost', s => s.writeBool(l.vectorQualityLost));
  s.writeField('color_space_fidelity_risk', s => s.writeBool(l.colorSpaceFidelityRisk));
  s.writeField('spot_colors_lost', s => s.writeBool(l.spotColorsLost));
  s.writeField('layers_lost', s => s.writeBool(l.layersLost));
  s.writeField('structure_association_lost', s => s.writeBool(l.structureAssociationLost));
  s.writeField('page_actions_lost', s => s.writeBool(l.pageActionsLost));
}

function serializeSecureRedactionPolicy(s, p) {
  s.writeObjectStart(9);
  s.writeField('schema_version', s => s.writeString(p.schemaVersion));
  s.writeField('profile', s => s.writeString('secure_rasterized_pages_v1'));
  s.writeField('strategy', s => s.writeString('rasterized_pages'));
  s.writeField('raster_dpi', s => s.writeU32(p.rasterDpi));
  s.writeField('appearance', s => {
    s.writeObjectStart(3);
    s.writeField('kind', s => s.writeString(p.appearance.kind));
    s.writeField('color', s => s.writeString(p.appearance.color));
    s.writeField('opacity', s => s.writeF64(p.appearance.opacity));
  });
  s.writeField('safety_margin_points', s => s.writeF64(p.safetyMarginPoints));
  s.writeField('interactive_content_policy', s => s.writeString(p.interactiveContentPolicy));
  s.writeField('signature_policy', s => s.writeString('block'));
  s.writeField('encryption_policy', s => s.writeString('block'));
}

function serializeRedactionAcknowledgements(s, a) {
  s.writeObjectStart(8);
  s.writeField('schema_version', s => s.writeString(a.schemaVersion));
  s.writeField('rasterized_pages_accepted', s => s.writeBool(a.rasterizedPagesAccepted));
  s.writeField('text_search_loss_accepted', s => s.writeBool(a.textSearchLossAccepted));
  s.writeField('accessibility_loss_accepted', s => s.writeBool(a.accessibilityLossAccepted));
  s.writeField('annotation_loss_accepted', s => s.writeBool(a.annotationLossAccepted));
  s.writeField('form_loss_accepted', s => s.writeBool(a.formLossAccepted));
  s.writeField('signature_invalidation_accepted', s => s.writeBool(a.signatureInvalidationAccepted));
  s.writeField('encryption_change_accepted', s => s.writeBool(a.encryptionChangeAccepted));
}

function serializeRedactionAcknowledgementRequirements(s, r) {
  s.writeObjectStart(8);
  s.writeField('schema_version', s => s.writeString(r.schemaVersion));
  s.writeField('rasterized_pages_required', s => s.writeBool(r.rasterizedPagesRequired));
  s.writeField('text_search_loss_required', s => s.writeBool(r.textSearchLossRequired));
  s.writeField('accessibility_loss_required', s => s.writeBool(r.accessibilityLossRequired));
  s.writeField('annotation_loss_required', s => s.writeBool(r.annotationLossRequired));
  s.writeField('form_loss_required', s => s.writeBool(r.formLossRequired));
  s.writeField('signature_invalidation_required', s => s.writeBool(r.signatureInvalidationRequired));
  s.writeField('encryption_change_required', s => s.writeBool(r.encryptionChangeRequired));
}

function serializePageRedactionPlan(s, p) {
  s.writeObjectStart(11);
  s.writeField('schema_version', s => s.writeString(p.schemaVersion));
  s.writeField('page_index', s => s.writeU32(p.pageIndex));
  s.writeField('strategy', s => s.writeString(p.strategy));
  s.writeField('geometry', s => writeCanonicalPageGeometryFields(s, p.geometry));
  s.writeField('geometry_digest', s => s.writeString(p.geometryDigest));
  s.writeField('canonical_regions', s => {
    s.writeListStart(p.canonicalRegions.length);
    for (const r of p.canonicalRegions) {
      serializeCanonicalRegion(s, r);
    }
  });
  s.writeField('facts', s => serializePageRedactionFacts(s, p.facts));
  s.writeField('feature_losses', s => serializeRedactionFeatureLosses(s, p.featureLosses));
  s.writeField('blockers', s => {
    s.writeListStart(p.blockers.length);
    for (const b of p.blockers) {
      serializeBlocker(s, b);
    }
  });
  s.writeField('warnings', s => {
    s.writeListStart(p.warnings.length);
    for (const w of p.warnings) {
      serializeWarning(s, w);
    }
  });
  s.writeField('executable', s => s.writeBool(p.executable));
}

export async function computePlanDigest(plan) {
  const s = new CanonicalSerializer();
  s.writeObjectStart(13);

  s.writeField('identity', s => {
    s.writeObjectStart(4);
    s.writeField('schema_version', s => s.writeString(REDACTION_SCHEMA_VERSION));
    s.writeField('plan_version', s => s.writeString(SECURE_REDACTION_PLAN_VERSION));
    s.writeField('input_digest', s => s.writeString(plan.inputDigest));
    s.writeField('content_generation', s => s.writeU64(plan.contentGeneration));
  });
  s.writeField('policy', s => serializeSecureRedactionPolicy(s, plan.policy));
  s.writeField('merge_profile', s => {
    s.writeObjectStart(4);
    s.writeField('adjacency_tolerance_points', s => s.writeF64(REDACTION_LIMITS.ADJACENCY_TOLERANCE_POINTS));
    s.writeField('merge_edge_touching', s => s.writeBool(REDACTION_LIMITS.MERGE_EDGE_TOUCHING));
    s.writeField('merge_corner_touching', s => s.writeBool(REDACTION_LIMITS.MERGE_CORNER_TOUCHING));
    s.writeField('geometry_comparison_epsilon', s => s.writeF64(GEOMETRY_EPSILON));
  });
  s.writeField('acknowledgements', s => serializeRedactionAcknowledgements(s, plan.acknowledgements));
  s.writeField('acknowledgement_requirements', s => serializeRedactionAcknowledgementRequirements(s, plan.acknowledgementRequirements));
  s.writeField('facts', s => serializeDocumentRedactionFacts(s, plan.facts));
  s.writeField('losses', s => serializeRedactionFeatureLosses(s, plan.losses));
  s.writeField('readiness', s => {
    s.writeObjectStart(3);
    s.writeField('executable', s => s.writeBool(plan.executable));
    s.writeField('acknowledgements_satisfied', s => s.writeBool(plan.acknowledgementsSatisfied));
    s.writeField('ready_for_execution', s => s.writeBool(plan.readyForExecution));
  });
  s.writeField('page_plans', s => {
    s.writeListStart(plan.pagePlans.length);
    for (const p of plan.pagePlans) {
      serializePageRedactionPlan(s, p);
    }
  });
  s.writeField('canonical_regions', s => {
    s.writeListStart(plan.canonicalRegions.length);
    for (const r of plan.canonicalRegions) {
      serializeCanonicalRegion(s, r);
    }
  });
  s.writeField('blockers', s => {
    s.writeListStart(plan.blockers.length);
    for (const b of plan.blockers) {
      serializeBlocker(s, b);
    }
  });
  s.writeField('warnings', s => {
    s.writeListStart(plan.warnings.length);
    for (const w of plan.warnings) {
      serializeWarning(s, w);
    }
  });

  return await s.finalizeDigest();
}

export async function verifySecureRedactionPlanDigests(plan) {
  for (const pagePlan of plan.pagePlans) {
    const recomputedGeomDigest = await computeGeometryDigest(pagePlan.geometry);
    if (pagePlan.geometryDigest !== recomputedGeomDigest) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_PAGE_GEOMETRY_MISMATCH,
        `geometryDigest (${pagePlan.geometryDigest}) does not match recomputed geometry digest (${recomputedGeomDigest})`
      );
    }
  }

  const recomputedPlanDigest = await computePlanDigest(plan);
  if (plan.planDigest !== recomputedPlanDigest) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_PLAN_DIGEST_MISMATCH,
      `planDigest (${plan.planDigest}) does not match recomputed plan digest (${recomputedPlanDigest})`
    );
  }

  return true;
}

export async function normalizeAndVerifySecureRedactionPlan(raw) {
  const plan = normalizeSecureRedactionPlanStructure(raw);
  await verifySecureRedactionPlanDigests(plan);
  return {
    ...plan,
    digestVerified: true,
  };
}

// ---------------------------------------------------------------------------
// Strict Type Assertions (NO COERCION, UTF-8 Byte Bounds, Numeric Rust Parity)
// ---------------------------------------------------------------------------

function assertStrictBoolean(val, fieldName, errorCode = REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY) {
  if (typeof val !== 'boolean') {
    throw new RedactionError(
      errorCode,
      `${fieldName} must be a boolean value, got ${val === null ? 'null' : typeof val}`
    );
  }
  return val;
}

export function assertSafeInteger(val, fieldName, errorCode = REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY) {
  if (typeof val !== 'number' || !Number.isFinite(val) || !Number.isInteger(val) || !Number.isSafeInteger(val)) {
    throw new RedactionError(
      errorCode,
      `${fieldName} must be a safe integer, got ${val === null ? 'null' : typeof val}`
    );
  }
  return val;
}

export function assertU32(val, fieldName, errorCode = REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY) {
  assertSafeInteger(val, fieldName, errorCode);
  if (val < U32_MIN || val > U32_MAX) {
    throw new RedactionError(
      errorCode,
      `${fieldName} must be a u32 integer between ${U32_MIN} and ${U32_MAX}, got ${val}`
    );
  }
  return val;
}

export function assertPositiveU32(val, fieldName, errorCode = REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY) {
  assertSafeInteger(val, fieldName, errorCode);
  if (val < 1 || val > U32_MAX) {
    throw new RedactionError(
      errorCode,
      `${fieldName} must be a positive u32 integer between 1 and ${U32_MAX}, got ${val}`
    );
  }
  return val;
}

export function assertI32(val, fieldName, errorCode = REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY) {
  assertSafeInteger(val, fieldName, errorCode);
  if (val < I32_MIN || val > I32_MAX) {
    throw new RedactionError(
      errorCode,
      `${fieldName} must be an i32 integer between ${I32_MIN} and ${I32_MAX}, got ${val}`
    );
  }
  return val;
}

function assertStrictNumber(val, fieldName, { integer = false, min = undefined, max = undefined } = {}, errorCode = REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY) {
  if (typeof val !== 'number' || !Number.isFinite(val)) {
    throw new RedactionError(
      errorCode,
      `${fieldName} must be a finite number, got ${val === null ? 'null' : typeof val}`
    );
  }
  if (integer) {
    assertSafeInteger(val, fieldName, errorCode);
  }
  if (min !== undefined && val < min) {
    throw new RedactionError(
      errorCode,
      `${fieldName} must be >= ${min}, got ${val}`
    );
  }
  if (max !== undefined && val > max) {
    throw new RedactionError(
      errorCode,
      `${fieldName} must be <= ${max}, got ${val}`
    );
  }
  return val;
}

function assertStrictString(val, fieldName, { minLen = 1, maxLen = 4096, allowEmpty = false } = {}, errorCode = REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY) {
  if (typeof val !== 'string') {
    throw new RedactionError(
      errorCode,
      `${fieldName} must be a string, got ${val === null ? 'null' : typeof val}`
    );
  }
  if (!allowEmpty && val.trim().length === 0) {
    throw new RedactionError(
      errorCode,
      `${fieldName} must be non-empty and non-whitespace`
    );
  }
  const byteLen = getUtf8ByteLength(val);
  if (byteLen > maxLen) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED,
      `${fieldName} byte length (${byteLen}) exceeds maximum limit of ${maxLen} UTF-8 bytes`
    );
  }
  return val;
}

function assertStrictNullableString(val, fieldName, maxLen = 4096) {
  if (val === null || val === undefined) {
    return null;
  }
  if (typeof val !== 'string') {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `${fieldName} must be a string or null, got ${typeof val}`
    );
  }
  const byteLen = getUtf8ByteLength(val);
  if (byteLen > maxLen) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED,
      `${fieldName} byte length (${byteLen}) exceeds maximum limit of ${maxLen} UTF-8 bytes`
    );
  }
  return val;
}

// ---------------------------------------------------------------------------
// 1. Normalized Top Left Rectangle & Region Input
// ---------------------------------------------------------------------------

/**
 * @typedef {Object} NormalizedTopLeftRect
 * @property {number} x
 * @property {number} y
 * @property {number} width
 * @property {number} height
 */

/**
 * @typedef {Object} RedactionRegionInput
 * @property {string} id
 * @property {number} pageIndex
 * @property {NormalizedTopLeftRect} normalizedTopLeftRect
 * @property {string} source
 */

export function normalizeNormalizedTopLeftRect(raw) {
  if (!raw || typeof raw !== 'object') {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_REGION,
      'NormalizedTopLeftRect must be an object'
    );
  }
  assertNoUnknownFields(raw, ['x', 'y', 'width', 'height'], 'NormalizedTopLeftRect');

  const x = assertStrictNumber(raw.x, 'x', {}, REDACTION_ERROR_CODES.REDACTION_INVALID_REGION);
  const y = assertStrictNumber(raw.y, 'y', {}, REDACTION_ERROR_CODES.REDACTION_INVALID_REGION);
  const width = assertStrictNumber(raw.width, 'width', {}, REDACTION_ERROR_CODES.REDACTION_INVALID_REGION);
  const height = assertStrictNumber(raw.height, 'height', {}, REDACTION_ERROR_CODES.REDACTION_INVALID_REGION);

  const rect = { x, y, width, height };
  validateNormalizedTopLeftRect(rect);
  return rect;
}

export function validateNormalizedTopLeftRect(rect) {
  if (!rect || typeof rect !== 'object' || Array.isArray(rect)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_REGION,
      'NormalizedTopLeftRect must be an object'
    );
  }
  assertNoUnknownFields(rect, ['x', 'y', 'width', 'height'], 'NormalizedTopLeftRect');
  assertStrictNumber(rect.x, 'x', {}, REDACTION_ERROR_CODES.REDACTION_INVALID_REGION);
  assertStrictNumber(rect.y, 'y', {}, REDACTION_ERROR_CODES.REDACTION_INVALID_REGION);
  assertStrictNumber(rect.width, 'width', {}, REDACTION_ERROR_CODES.REDACTION_INVALID_REGION);
  assertStrictNumber(rect.height, 'height', {}, REDACTION_ERROR_CODES.REDACTION_INVALID_REGION);

  if (rect.width <= 0 || rect.height <= 0) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_REGION,
      'Normalized rectangle width and height must be strictly positive (> 0)'
    );
  }

  const maxX = rect.x + rect.width;
  const maxY = rect.y + rect.height;
  if (!Number.isFinite(maxX) || !Number.isFinite(maxY)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_REGION,
      'Rectangle addition overflow or non-finite result'
    );
  }

  // Exact bounds check without epsilon or arbitrary cutoff (tiny positive floats like 1e-8 allowed)
  if (rect.x < 0 || rect.y < 0 || maxX > 1.0 || maxY > 1.0) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_REGION_OUT_OF_BOUNDS,
      'Normalized rectangle is out of page bounds [0.0, 1.0]'
    );
  }
}

export function normalizeRedactionRegionInput(raw) {
  if (!raw || typeof raw !== 'object') {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_REGION,
      'RedactionRegionInput must be an object'
    );
  }

  assertNoUnknownFields(
    raw,
    ['id', 'pageIndex', 'page_index', 'normalizedTopLeftRect', 'normalized_top_left_rect', 'source'],
    'RedactionRegionInput'
  );

  const id = assertStrictString(raw.id, 'id', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_REGION_ID_LEN }, REDACTION_ERROR_CODES.REDACTION_INVALID_REGION);
  const pageIndexRaw = readExclusiveAlias(raw, 'pageIndex', 'page_index', true, 'pageIndex', 'RedactionRegionInput');
  const pageIndex = assertU32(pageIndexRaw, 'pageIndex', REDACTION_ERROR_CODES.REDACTION_INVALID_REGION);

  const rectRaw = readExclusiveAlias(raw, 'normalizedTopLeftRect', 'normalized_top_left_rect', true, 'normalizedTopLeftRect', 'RedactionRegionInput');
  const normalizedTopLeftRect = normalizeNormalizedTopLeftRect(rectRaw);
  const source = assertStrictString(raw.source, 'source', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_SOURCE_LEN }, REDACTION_ERROR_CODES.REDACTION_INVALID_REGION);

  return { id, pageIndex, normalizedTopLeftRect, source };
}

export function validateRedactionRegionInput(rawOrInput) {
  normalizeRedactionRegionInput(rawOrInput);
  return true;
}

export function validateRedactionRegionInputs(rawInputs) {
  if (!Array.isArray(rawInputs)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_REGION,
      'Inputs must be an array of RedactionRegionInput'
    );
  }

  if (rawInputs.length > REDACTION_LIMITS.MAX_REGIONS_PER_INPUT) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_REGION_LIMIT_EXCEEDED,
      `Region count (${rawInputs.length}) exceeds maximum limit of ${REDACTION_LIMITS.MAX_REGIONS_PER_INPUT}`
    );
  }

  const seenIds = new Set();
  return rawInputs.map(raw => {
    const input = normalizeRedactionRegionInput(raw);
    validateRedactionRegionInput(input);
    if (seenIds.has(input.id)) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_DUPLICATE_REGION_ID,
        `Duplicate region ID found: '${input.id}'`
      );
    }
    seenIds.add(input.id);
    return input;
  });
}

// ---------------------------------------------------------------------------
// 2. Canonical Internal Models (Frontend)
// ---------------------------------------------------------------------------

/**
 * @typedef {Object} PdfPointRect
 * @property {number} left
 * @property {number} bottom
 * @property {number} width
 * @property {number} height
 */

/**
 * @typedef {Object} DevicePixelRect
 * @property {number} left
 * @property {number} top
 * @property {number} width
 * @property {number} height
 */

/**
 * @typedef {Object} CanonicalPageGeometry
 * @property {number} pageIndex
 * @property {PdfPointRect} mediaBox
 * @property {PdfPointRect|null} cropBox
 * @property {PdfPointRect} effectiveBox
 * @property {0|90|180|270} rotation
 */

/**
 * @typedef {Object} CanonicalRedactionRegion
 * @property {string} regionId
 * @property {string[]} sourceRegionIds
 * @property {number} pageIndex
 * @property {NormalizedTopLeftRect} originalNormalizedRect
 * @property {PdfPointRect} canonicalPdfPointRect
 * @property {number} appliedSafetyMarginPoints
 * @property {DevicePixelRect|null} devicePixelRect
 */

export function normalizePdfPointRect(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'PdfPointRect must be an object');
  }
  assertNoUnknownFields(raw, ['left', 'bottom', 'width', 'height'], 'PdfPointRect');
  const left = assertStrictNumber(raw.left, 'left');
  const bottom = assertStrictNumber(raw.bottom, 'bottom');
  const width = assertStrictNumber(raw.width, 'width');
  const height = assertStrictNumber(raw.height, 'height');

  if (width <= 0 || height <= 0) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'PdfPointRect width and height must be strictly positive (> 0)');
  }
  if (!Number.isFinite(left + width) || !Number.isFinite(bottom + height)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'PdfPointRect addition overflow');
  }
  return { left, bottom, width, height };
}

export function assertI32PlusU32DoesNotOverflow(origin, extent, originName, extentName) {
  assertI32(origin, originName);
  assertPositiveU32(extent, extentName);
  const endExclusive = origin + extent;
  if (!Number.isSafeInteger(endExclusive) || endExclusive < I32_MIN || endExclusive > I32_MAX) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `DevicePixelRect coordinate arithmetic overflow: ${originName} (${origin}) + ${extentName} (${extent}) = ${endExclusive} is out of i32 bounds`
    );
  }
  return endExclusive;
}

export function normalizeDevicePixelRect(raw) {
  if (raw === null || raw === undefined) return null;
  if (typeof raw !== 'object' || Array.isArray(raw)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'DevicePixelRect must be an object');
  }
  assertNoUnknownFields(raw, ['left', 'top', 'width', 'height'], 'DevicePixelRect');
  const left = assertI32(raw.left, 'left');
  const top = assertI32(raw.top, 'top');
  const width = assertPositiveU32(raw.width, 'width');
  const height = assertPositiveU32(raw.height, 'height');

  assertI32PlusU32DoesNotOverflow(left, width, 'left', 'width');
  assertI32PlusU32DoesNotOverflow(top, height, 'top', 'height');

  return { left, top, width, height };
}

export function validateCanonicalPageGeometry(geom) {
  const eps = GEOMETRY_EPSILON;
  normalizePdfPointRect(geom.mediaBox);
  if (geom.cropBox) {
    normalizePdfPointRect(geom.cropBox);
    if (
      geom.cropBox.left < geom.mediaBox.left - eps ||
      geom.cropBox.bottom < geom.mediaBox.bottom - eps ||
      geom.cropBox.left + geom.cropBox.width > geom.mediaBox.left + geom.mediaBox.width + eps ||
      geom.cropBox.bottom + geom.cropBox.height > geom.mediaBox.bottom + geom.mediaBox.height + eps
    ) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_PAGE_GEOMETRY_MISMATCH,
        'CropBox extends outside MediaBox boundaries (unsupported geometry)'
      );
    }
  }
  for (const [label, box] of [['TrimBox', geom.trimBox], ['BleedBox', geom.bleedBox], ['ArtBox', geom.artBox]]) {
    if (box) {
      try {
        normalizePdfPointRect(box);
      } catch {
        throw new RedactionError(
          REDACTION_ERROR_CODES.REDACTION_PAGE_GEOMETRY_MISMATCH,
          `${label} is not a valid finite PDF rectangle`
        );
      }
    }
  }
  normalizePdfPointRect(geom.effectiveBox);

  const expectedBox = geom.cropBox || geom.mediaBox;
  if (
    Math.abs(geom.effectiveBox.left - expectedBox.left) > eps ||
    Math.abs(geom.effectiveBox.bottom - expectedBox.bottom) > eps ||
    Math.abs(geom.effectiveBox.width - expectedBox.width) > eps ||
    Math.abs(geom.effectiveBox.height - expectedBox.height) > eps
  ) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_PAGE_GEOMETRY_MISMATCH,
      'effective_box must match CropBox (if present) or MediaBox (if CropBox absent)'
    );
  }
}

export function normalizeCanonicalPageGeometry(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'CanonicalPageGeometry must be an object');
  }
  assertNoUnknownFields(
    raw,
    ['pageIndex', 'page_index', 'mediaBox', 'media_box', 'cropBox', 'crop_box', 'trimBox', 'trim_box', 'bleedBox', 'bleed_box', 'artBox', 'art_box', 'effectiveBox', 'effective_box', 'rotation'],
    'CanonicalPageGeometry'
  );

  const pageIndexRaw = readExclusiveAlias(raw, 'pageIndex', 'page_index', true, 'pageIndex', 'CanonicalPageGeometry');
  const pageIndex = assertU32(pageIndexRaw, 'pageIndex');

  const mediaBoxRaw = readExclusiveAlias(raw, 'mediaBox', 'media_box', true, 'mediaBox', 'CanonicalPageGeometry');
  const mediaBox = normalizePdfPointRect(mediaBoxRaw);

  const cropBoxRaw = readExclusiveAlias(raw, 'cropBox', 'crop_box', false, 'cropBox', 'CanonicalPageGeometry');
  const cropBox = normalizeNullableObject(cropBoxRaw, normalizePdfPointRect, 'cropBox');

  const trimBoxRaw = readExclusiveAlias(raw, 'trimBox', 'trim_box', false, 'trimBox', 'CanonicalPageGeometry');
  const trimBox = normalizeNullableObject(trimBoxRaw, normalizePdfPointRect, 'trimBox');
  const bleedBoxRaw = readExclusiveAlias(raw, 'bleedBox', 'bleed_box', false, 'bleedBox', 'CanonicalPageGeometry');
  const bleedBox = normalizeNullableObject(bleedBoxRaw, normalizePdfPointRect, 'bleedBox');
  const artBoxRaw = readExclusiveAlias(raw, 'artBox', 'art_box', false, 'artBox', 'CanonicalPageGeometry');
  const artBox = normalizeNullableObject(artBoxRaw, normalizePdfPointRect, 'artBox');

  const effectiveBoxRaw = readExclusiveAlias(raw, 'effectiveBox', 'effective_box', true, 'effectiveBox', 'CanonicalPageGeometry');
  const effectiveBox = normalizePdfPointRect(effectiveBoxRaw);

  const rotation = assertStrictNumber(raw.rotation, 'rotation', { integer: true });
  if (![0, 90, 180, 270].includes(rotation)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_PAGE_GEOMETRY_MISMATCH,
      `Invalid page rotation angle: ${rotation}. Expected 0, 90, 180, or 270`
    );
  }

  const geom = { pageIndex, mediaBox, cropBox, trimBox, bleedBox, artBox, effectiveBox, rotation };
  validateCanonicalPageGeometry(geom);
  return geom;
}

export function normalizedTopLeftToPdfPointRect(norm, geom) {
  const boxEff = geom.effectiveBox;
  const pageTop = boxEff.bottom + boxEff.height;
  const pdfLeft = boxEff.left + norm.x * boxEff.width;
  const pdfBottom = pageTop - (norm.y + norm.height) * boxEff.height;
  const pdfWidth = norm.width * boxEff.width;
  const pdfHeight = norm.height * boxEff.height;
  return {
    left: pdfLeft,
    bottom: pdfBottom,
    width: pdfWidth,
    height: pdfHeight,
  };
}

export function validateCanonicalRedactionRegionInvariants(region, pageGeometry, policySafetyMargin) {
  const eps = GEOMETRY_EPSILON;
  const m = region.appliedMargins;
  const reqM = region.requestedSafetyMarginPoints;

  if (policySafetyMargin !== undefined && Math.abs(reqM - policySafetyMargin) > eps) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `requestedSafetyMarginPoints (${reqM}) does not match policy safety margin (${policySafetyMargin})`
    );
  }

  if (m.left < 0 || m.right < 0 || m.top < 0 || m.bottom < 0) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'Applied margin values must be >= 0.0'
    );
  }

  if (m.left > reqM + eps || m.right > reqM + eps || m.top > reqM + eps || m.bottom > reqM + eps) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'Applied margin exceeds requested safety margin beyond tolerance'
    );
  }

  const base = region.basePdfPointRect;
  const applied = region.appliedPdfPointRect;

  if (
    applied.left > base.left + eps ||
    applied.bottom > base.bottom + eps ||
    applied.left + applied.width < base.left + base.width - eps ||
    applied.bottom + applied.height < base.bottom + base.height - eps
  ) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'Applied rectangle does not fully cover base rectangle'
    );
  }

  if (
    Math.abs(applied.left - (base.left - m.left)) > eps ||
    Math.abs(applied.bottom - (base.bottom - m.bottom)) > eps ||
    Math.abs((applied.left + applied.width) - (base.left + base.width + m.right)) > eps ||
    Math.abs((applied.bottom + applied.height) - (base.bottom + base.height + m.top)) > eps
  ) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'Applied rectangle does not match base rectangle and applied margins'
    );
  }

  if (reqM === 0) {
    if (m.left > eps || m.right > eps || m.top > eps || m.bottom > eps || region.boundaryAdjusted) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        'Requested safety margin is 0.0 but applied margins are non-zero or boundary_adjusted is true'
      );
    }
  } else if (!region.boundaryAdjusted) {
    if (
      Math.abs(m.left - reqM) > eps ||
      Math.abs(m.right - reqM) > eps ||
      Math.abs(m.top - reqM) > eps ||
      Math.abs(m.bottom - reqM) > eps
    ) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        'boundary_adjusted is false but applied margins differ from requested margin'
      );
    }
  } else {
    if (
      m.left >= reqM - eps &&
      m.right >= reqM - eps &&
      m.top >= reqM - eps &&
      m.bottom >= reqM - eps
    ) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        'boundary_adjusted is true but all applied margins equal requested margin'
      );
    }
  }

  if (pageGeometry) {
    const expectedBase = normalizedTopLeftToPdfPointRect(region.originalNormalizedRect, pageGeometry);
    if (
      Math.abs(base.left - expectedBase.left) > eps ||
      Math.abs(base.bottom - expectedBase.bottom) > eps ||
      Math.abs(base.width - expectedBase.width) > eps ||
      Math.abs(base.height - expectedBase.height) > eps
    ) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_PAGE_GEOMETRY_MISMATCH,
        'basePdfPointRect does not match recomputed rect from originalNormalizedRect and page geometry'
      );
    }

    const eff = pageGeometry.effectiveBox;
    if (
      base.left < eff.left - eps ||
      base.bottom < eff.bottom - eps ||
      base.left + base.width > eff.left + eff.width + eps ||
      base.bottom + base.height > eff.bottom + eff.height + eps
    ) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_PAGE_GEOMETRY_MISMATCH,
        'basePdfPointRect extends outside effectiveBox'
      );
    }

    if (
      applied.left < eff.left - eps ||
      applied.bottom < eff.bottom - eps ||
      applied.left + applied.width > eff.left + eff.width + eps ||
      applied.bottom + applied.height > eff.bottom + eff.height + eps
    ) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_PAGE_GEOMETRY_MISMATCH,
        'appliedPdfPointRect extends outside effectiveBox'
      );
    }
  }
}

export function normalizeInspectionFactState(raw, fieldName = 'factState') {
  if (raw === 'present' || raw === 'absent' || raw === 'unknown') {
    return raw;
  }
  throw new RedactionError(
    REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
    `Invalid inspection fact state for '${fieldName}': expected 'present', 'absent', or 'unknown', got '${raw}'`
  );
}

export function normalizeAppliedMargins(raw, parentName = 'CanonicalRedactionRegion') {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_REGION,
      `appliedMargins must be an object in ${parentName}`
    );
  }
  assertNoUnknownFields(raw, ['left', 'right', 'top', 'bottom'], `${parentName}.appliedMargins`);
  const left = assertStrictNumber(raw.left, 'left', { min: 0 });
  const right = assertStrictNumber(raw.right, 'right', { min: 0 });
  const top = assertStrictNumber(raw.top, 'top', { min: 0 });
  const bottom = assertStrictNumber(raw.bottom, 'bottom', { min: 0 });
  return { left, right, top, bottom };
}

export function normalizeCanonicalRedactionRegion(raw, pageGeometry, policySafetyMargin) {
  if (!raw || typeof raw !== 'object') {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_REGION, 'CanonicalRedactionRegion must be an object');
  }
  assertNoUnknownFields(
    raw,
    [
      'regionId', 'region_id',
      'sourceRegionIds', 'source_region_ids',
      'pageIndex', 'page_index',
      'originalNormalizedRect', 'original_normalized_rect',
      'basePdfPointRect', 'base_pdf_point_rect',
      'requestedSafetyMarginPoints', 'requested_safety_margin_points',
      'appliedPdfPointRect', 'applied_pdf_point_rect',
      'appliedMargins', 'applied_margins',
      'boundaryAdjusted', 'boundary_adjusted',
      'devicePixelRect', 'device_pixel_rect',
    ],
    'CanonicalRedactionRegion'
  );

  const regionIdRaw = readExclusiveAlias(raw, 'regionId', 'region_id', true, 'regionId', 'CanonicalRedactionRegion');
  const regionId = assertStrictString(regionIdRaw, 'regionId', { maxLen: REDACTION_LIMITS.MAX_REGION_ID_LEN }, REDACTION_ERROR_CODES.REDACTION_INVALID_REGION);

  const rawSources = readExclusiveAlias(raw, 'sourceRegionIds', 'source_region_ids', true, 'sourceRegionIds', 'CanonicalRedactionRegion');
  if (!Array.isArray(rawSources) || rawSources.length === 0) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_REGION, 'sourceRegionIds must be a non-empty array');
  }
  if (rawSources.length > REDACTION_LIMITS.MAX_SOURCE_REGION_IDS) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED, 'sourceRegionIds exceeds max limit');
  }
  const seenSources = new Set();
  let prevSourceId = null;
  const sourceRegionIds = rawSources.map(s => {
    const idStr = assertStrictString(s, 'sourceRegionId', { maxLen: REDACTION_LIMITS.MAX_REGION_ID_LEN }, REDACTION_ERROR_CODES.REDACTION_INVALID_REGION);
    if (seenSources.has(idStr)) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_DUPLICATE_REGION_ID,
        `Duplicate sourceRegionId '${idStr}' in canonical region`
      );
    }
    if (prevSourceId !== null && compareCanonicalUtf8Strings(prevSourceId, idStr) >= 0) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        `sourceRegionIds must be in lexicographically sorted order ('${idStr}' came after '${prevSourceId}')`
      );
    }
    seenSources.add(idStr);
    prevSourceId = idStr;
    return idStr;
  });

  const pageIndexRaw = readExclusiveAlias(raw, 'pageIndex', 'page_index', true, 'pageIndex', 'CanonicalRedactionRegion');
  const pageIndex = assertU32(pageIndexRaw, 'pageIndex', REDACTION_ERROR_CODES.REDACTION_INVALID_REGION);

  const origRectRaw = readExclusiveAlias(raw, 'originalNormalizedRect', 'original_normalized_rect', true, 'originalNormalizedRect', 'CanonicalRedactionRegion');
  const originalNormalizedRect = normalizeNormalizedTopLeftRect(origRectRaw);

  const basePdfRaw = readExclusiveAlias(raw, 'basePdfPointRect', 'base_pdf_point_rect', true, 'basePdfPointRect', 'CanonicalRedactionRegion');
  const basePdfPointRect = normalizePdfPointRect(basePdfRaw);

  const requestedMarginRaw = readExclusiveAlias(raw, 'requestedSafetyMarginPoints', 'requested_safety_margin_points', true, 'requestedSafetyMarginPoints', 'CanonicalRedactionRegion');
  const requestedSafetyMarginPoints = assertStrictNumber(requestedMarginRaw, 'requestedSafetyMarginPoints', { min: REDACTION_LIMITS.MIN_SAFETY_MARGIN_POINTS, max: REDACTION_LIMITS.MAX_SAFETY_MARGIN_POINTS });

  const appliedPdfRaw = readExclusiveAlias(raw, 'appliedPdfPointRect', 'applied_pdf_point_rect', true, 'appliedPdfPointRect', 'CanonicalRedactionRegion');
  const appliedPdfPointRect = normalizePdfPointRect(appliedPdfRaw);

  const appliedMarginsRaw = readExclusiveAlias(raw, 'appliedMargins', 'applied_margins', true, 'appliedMargins', 'CanonicalRedactionRegion');
  const appliedMargins = normalizeAppliedMargins(appliedMarginsRaw, 'CanonicalRedactionRegion');

  const boundaryAdjRaw = readExclusiveAlias(raw, 'boundaryAdjusted', 'boundary_adjusted', true, 'boundaryAdjusted', 'CanonicalRedactionRegion');
  const boundaryAdjusted = assertStrictBoolean(boundaryAdjRaw, 'boundaryAdjusted');

  const devPixelRaw = readExclusiveAlias(raw, 'devicePixelRect', 'device_pixel_rect', false, 'devicePixelRect', 'CanonicalRedactionRegion');
  const devicePixelRect = normalizeDevicePixelRect(devPixelRaw);

  const region = {
    regionId,
    sourceRegionIds,
    pageIndex,
    originalNormalizedRect,
    basePdfPointRect,
    requestedSafetyMarginPoints,
    appliedPdfPointRect,
    appliedMargins,
    boundaryAdjusted,
    devicePixelRect,
  };

  validateCanonicalRedactionRegionInvariants(region, pageGeometry, policySafetyMargin);
  return region;
}

// ---------------------------------------------------------------------------
// 3. Policy Contract
// ---------------------------------------------------------------------------

/**
 * @typedef {Object} RedactionAppearancePolicy
 * @property {'solid'} kind
 * @property {'black'} color
 * @property {1.0} opacity
 */

/**
 * @typedef {Object} SecureRedactionPolicy
 * @property {string} schemaVersion
 * @property {'secure_rasterized_pages_v1'} profile
 * @property {'rasterized_pages'} strategy
 * @property {number} rasterDpi
 * @property {RedactionAppearancePolicy} appearance
 * @property {number} safetyMarginPoints
 * @property {'strip'|'block'} interactiveContentPolicy
 * @property {'block'} signaturePolicy
 * @property {'block'} encryptionPolicy
 */

export function normalizeSecureRedactionPolicy(raw) {
  if (!raw || typeof raw !== 'object') {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'SecureRedactionPolicy must be an object'
    );
  }

  assertNoUnknownFields(
    raw,
    [
      'schemaVersion', 'schema_version',
      'profile',
      'strategy',
      'rasterDpi', 'raster_dpi',
      'appearance',
      'safetyMarginPoints', 'safety_margin_points',
      'interactiveContentPolicy', 'interactive_content_policy',
      'signaturePolicy', 'signature_policy',
      'encryptionPolicy', 'encryption_policy',
    ],
    'SecureRedactionPolicy'
  );

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'SecureRedactionPolicy');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED,
      `Unsupported policy schema version: '${schemaVersion}'`
    );
  }

  const profile = assertStrictString(raw.profile, 'profile');
  const strategy = assertStrictString(raw.strategy, 'strategy');

  const rasterDpiRaw = readExclusiveAlias(raw, 'rasterDpi', 'raster_dpi', true, 'rasterDpi', 'SecureRedactionPolicy');
  const rasterDpi = assertStrictNumber(rasterDpiRaw, 'rasterDpi', { integer: true, min: REDACTION_LIMITS.MIN_RASTER_DPI, max: REDACTION_LIMITS.MAX_RASTER_DPI });

  const safetyMarginRaw = readExclusiveAlias(raw, 'safetyMarginPoints', 'safety_margin_points', true, 'safetyMarginPoints', 'SecureRedactionPolicy');
  const safetyMarginPoints = assertStrictNumber(safetyMarginRaw, 'safetyMarginPoints', { min: REDACTION_LIMITS.MIN_SAFETY_MARGIN_POINTS, max: REDACTION_LIMITS.MAX_SAFETY_MARGIN_POINTS });

  const interactiveRaw = readExclusiveAlias(raw, 'interactiveContentPolicy', 'interactive_content_policy', true, 'interactiveContentPolicy', 'SecureRedactionPolicy');
  const interactiveContentPolicy = assertStrictString(interactiveRaw, 'interactiveContentPolicy');

  const signatureRaw = readExclusiveAlias(raw, 'signaturePolicy', 'signature_policy', true, 'signaturePolicy', 'SecureRedactionPolicy');
  const signaturePolicy = assertStrictString(signatureRaw, 'signaturePolicy');

  const encryptionRaw = readExclusiveAlias(raw, 'encryptionPolicy', 'encryption_policy', true, 'encryptionPolicy', 'SecureRedactionPolicy');
  const encryptionPolicy = assertStrictString(encryptionRaw, 'encryptionPolicy');

  const rawApp = raw.appearance;
  if (!rawApp || typeof rawApp !== 'object') {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'appearance must be an object'
    );
  }
  assertNoUnknownFields(rawApp, ['kind', 'color', 'opacity'], 'RedactionAppearancePolicy');

  const kind = assertStrictString(rawApp.kind, 'kind');
  const color = assertStrictString(rawApp.color, 'color');
  const opacity = assertStrictNumber(rawApp.opacity, 'opacity');

  const policy = {
    schemaVersion,
    profile,
    strategy,
    rasterDpi,
    appearance: { kind, color, opacity },
    safetyMarginPoints,
    interactiveContentPolicy,
    signaturePolicy,
    encryptionPolicy,
  };

  assertNormalizedSecureRedactionPolicy(policy);
  return policy;
}

function assertNormalizedSecureRedactionPolicy(policy) {
  if (policy.profile !== 'secure_rasterized_pages_v1') {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'Unsupported redaction profile'
    );
  }
  if (policy.strategy !== 'rasterized_pages') {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'Unsupported redaction strategy'
    );
  }
  const app = policy.appearance;
  if (app.kind !== 'solid' || app.color !== 'black' || app.opacity !== 1.0) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'First version supports only appearance: kind="solid", color="black", opacity=1.0'
    );
  }
  if (!['strip', 'block'].includes(policy.interactiveContentPolicy)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'interactiveContentPolicy must be "strip" or "block"'
    );
  }
  if (policy.signaturePolicy !== 'block') {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'signaturePolicy must be "block"'
    );
  }
  if (policy.encryptionPolicy !== 'block') {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'encryptionPolicy must be "block"'
    );
  }
}

export function validateSecureRedactionPolicy(rawOrPolicy) {
  normalizeSecureRedactionPolicy(rawOrPolicy);
  return true;
}

// ---------------------------------------------------------------------------
// 4. Acknowledgements Contract
// ---------------------------------------------------------------------------

/**
 * @typedef {Object} RedactionAcknowledgements
 * @property {string} schemaVersion
 * @property {boolean} rasterizedPagesAccepted
 * @property {boolean} textSearchLossAccepted
 * @property {boolean} accessibilityLossAccepted
 * @property {boolean} annotationLossAccepted
 * @property {boolean} formLossAccepted
 * @property {boolean} signatureInvalidationAccepted
 * @property {boolean} encryptionChangeAccepted
 */

export function normalizeRedactionAcknowledgements(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'RedactionAcknowledgements must be an object'
    );
  }

  assertNoUnknownFields(
    raw,
    [
      'schemaVersion', 'schema_version',
      'rasterizedPagesAccepted', 'rasterized_pages_accepted',
      'textSearchLossAccepted', 'text_search_loss_accepted',
      'accessibilityLossAccepted', 'accessibility_loss_accepted',
      'annotationLossAccepted', 'annotation_loss_accepted',
      'formLossAccepted', 'form_loss_accepted',
      'signatureInvalidationAccepted', 'signature_invalidation_accepted',
      'encryptionChangeAccepted', 'encryption_change_accepted',
    ],
    'RedactionAcknowledgements'
  );

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'RedactionAcknowledgements');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED,
      `Unsupported acknowledgements schema version: '${schemaVersion}'`
    );
  }

  return {
    schemaVersion,
    rasterizedPagesAccepted: assertStrictBoolean(readExclusiveAlias(raw, 'rasterizedPagesAccepted', 'rasterized_pages_accepted', true, 'rasterizedPagesAccepted', 'RedactionAcknowledgements'), 'rasterizedPagesAccepted'),
    textSearchLossAccepted: assertStrictBoolean(readExclusiveAlias(raw, 'textSearchLossAccepted', 'text_search_loss_accepted', true, 'textSearchLossAccepted', 'RedactionAcknowledgements'), 'textSearchLossAccepted'),
    accessibilityLossAccepted: assertStrictBoolean(readExclusiveAlias(raw, 'accessibilityLossAccepted', 'accessibility_loss_accepted', true, 'accessibilityLossAccepted', 'RedactionAcknowledgements'), 'accessibilityLossAccepted'),
    annotationLossAccepted: assertStrictBoolean(readExclusiveAlias(raw, 'annotationLossAccepted', 'annotation_loss_accepted', true, 'annotationLossAccepted', 'RedactionAcknowledgements'), 'annotationLossAccepted'),
    formLossAccepted: assertStrictBoolean(readExclusiveAlias(raw, 'formLossAccepted', 'form_loss_accepted', true, 'formLossAccepted', 'RedactionAcknowledgements'), 'formLossAccepted'),
    signatureInvalidationAccepted: assertStrictBoolean(readExclusiveAlias(raw, 'signatureInvalidationAccepted', 'signature_invalidation_accepted', true, 'signatureInvalidationAccepted', 'RedactionAcknowledgements'), 'signatureInvalidationAccepted'),
    encryptionChangeAccepted: assertStrictBoolean(readExclusiveAlias(raw, 'encryptionChangeAccepted', 'encryption_change_accepted', true, 'encryptionChangeAccepted', 'RedactionAcknowledgements'), 'encryptionChangeAccepted'),
  };
}

export function validateRedactionAcknowledgements(rawOrAck) {
  normalizeRedactionAcknowledgements(rawOrAck);
  return true;
}

export function normalizeRedactionAcknowledgementRequirements(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'RedactionAcknowledgementRequirements must be an object'
    );
  }

  assertNoUnknownFields(
    raw,
    [
      'schemaVersion', 'schema_version',
      'rasterizedPagesRequired', 'rasterized_pages_required',
      'textSearchLossRequired', 'text_search_loss_required',
      'accessibilityLossRequired', 'accessibility_loss_required',
      'annotationLossRequired', 'annotation_loss_required',
      'formLossRequired', 'form_loss_required',
      'signatureInvalidationRequired', 'signature_invalidation_required',
      'encryptionChangeRequired', 'encryption_change_required',
    ],
    'RedactionAcknowledgementRequirements'
  );

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'RedactionAcknowledgementRequirements');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED,
      `Unsupported acknowledgement requirements schema version: '${schemaVersion}'`
    );
  }

  return {
    schemaVersion,
    rasterizedPagesRequired: assertStrictBoolean(readExclusiveAlias(raw, 'rasterizedPagesRequired', 'rasterized_pages_required', true, 'rasterizedPagesRequired', 'RedactionAcknowledgementRequirements'), 'rasterizedPagesRequired'),
    textSearchLossRequired: assertStrictBoolean(readExclusiveAlias(raw, 'textSearchLossRequired', 'text_search_loss_required', true, 'textSearchLossRequired', 'RedactionAcknowledgementRequirements'), 'textSearchLossRequired'),
    accessibilityLossRequired: assertStrictBoolean(readExclusiveAlias(raw, 'accessibilityLossRequired', 'accessibility_loss_required', true, 'accessibilityLossRequired', 'RedactionAcknowledgementRequirements'), 'accessibilityLossRequired'),
    annotationLossRequired: assertStrictBoolean(readExclusiveAlias(raw, 'annotationLossRequired', 'annotation_loss_required', true, 'annotationLossRequired', 'RedactionAcknowledgementRequirements'), 'annotationLossRequired'),
    formLossRequired: assertStrictBoolean(readExclusiveAlias(raw, 'formLossRequired', 'form_loss_required', true, 'formLossRequired', 'RedactionAcknowledgementRequirements'), 'formLossRequired'),
    signatureInvalidationRequired: assertStrictBoolean(readExclusiveAlias(raw, 'signatureInvalidationRequired', 'signature_invalidation_required', true, 'signatureInvalidationRequired', 'RedactionAcknowledgementRequirements'), 'signatureInvalidationRequired'),
    encryptionChangeRequired: assertStrictBoolean(readExclusiveAlias(raw, 'encryptionChangeRequired', 'encryption_change_required', true, 'encryptionChangeRequired', 'RedactionAcknowledgementRequirements'), 'encryptionChangeRequired'),
  };
}

export function validateRedactionAcknowledgementRequirements(rawOrReq) {
  normalizeRedactionAcknowledgementRequirements(rawOrReq);
  return true;
}

// ---------------------------------------------------------------------------
// 5. Blockers, Warnings & Typed Verification Checks
// ---------------------------------------------------------------------------

/**
 * @typedef {Object} RedactionBlocker
 * @property {string} schemaVersion
 * @property {string} code
 * @property {string} message
 * @property {string|null} [details]
 */

/**
 * @typedef {Object} RedactionWarning
 * @property {string} schemaVersion
 * @property {string} code
 * @property {string} message
 * @property {string|null} [details]
 */

/**
 * @typedef {Object} RedactionVerificationCheck
 * @property {string} schemaVersion
 * @property {string} code
 * @property {'passed'|'failed'|'not_run'} status
 * @property {string} message
 * @property {string|null} [details]
 */

/**
 * @typedef {Object} SecureRedactionPublicationReport
 * @property {string} schemaVersion
 * @property {string} publicationId
 * @property {string} publishedPath
 * @property {string} publishedDigest
 * @property {'new_committed'|'new_committed_durability_unconfirmed'} commitState
 */

export function normalizeRedactionBlocker(raw) {
  if (!raw || typeof raw !== 'object') {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'RedactionBlocker must be an object');
  }
  assertNoUnknownFields(raw, ['schemaVersion', 'schema_version', 'code', 'message', 'details'], 'RedactionBlocker');

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'RedactionBlocker');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED, `Unsupported blocker schemaVersion: '${schemaVersion}'`);
  }

  const code = assertStrictString(raw.code, 'code', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_BLOCKER_CODE_LEN });
  const message = assertStrictString(raw.message, 'message', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_USER_SAFE_MESSAGE_LEN });
  const details = assertStrictNullableString(raw.details, 'details', REDACTION_LIMITS.MAX_DETAILS_LEN);

  return { schemaVersion, code, message, details };
}

export function normalizeRedactionWarning(raw) {
  if (!raw || typeof raw !== 'object') {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'RedactionWarning must be an object');
  }
  assertNoUnknownFields(raw, ['schemaVersion', 'schema_version', 'code', 'message', 'details'], 'RedactionWarning');

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'RedactionWarning');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED, `Unsupported warning schemaVersion: '${schemaVersion}'`);
  }

  const code = assertStrictString(raw.code, 'code', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_WARNING_CODE_LEN });
  const message = assertStrictString(raw.message, 'message', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_USER_SAFE_MESSAGE_LEN });
  const details = assertStrictNullableString(raw.details, 'details', REDACTION_LIMITS.MAX_DETAILS_LEN);

  return { schemaVersion, code, message, details };
}

export function normalizeRedactionVerificationCheck(raw) {
  if (!raw || typeof raw !== 'object') {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'RedactionVerificationCheck must be an object');
  }
  assertNoUnknownFields(raw, ['schemaVersion', 'schema_version', 'code', 'status', 'message', 'details'], 'RedactionVerificationCheck');

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'RedactionVerificationCheck');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED, `Unsupported verification check schemaVersion: '${schemaVersion}'`);
  }

  const code = assertStrictString(raw.code, 'code', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_VERIFICATION_CHECK_CODE_LEN });
  const status = assertStrictString(raw.status, 'status');
  if (!['passed', 'failed', 'not_run'].includes(status)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `Verification check status must be 'passed', 'failed', or 'not_run', got '${status}'`
    );
  }
  const message = assertStrictString(raw.message, 'message', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_VERIFICATION_CHECK_MESSAGE_LEN });
  const details = assertStrictNullableString(raw.details, 'details', REDACTION_LIMITS.MAX_VERIFICATION_CHECK_DETAILS_LEN);

  return { schemaVersion, code, status, message, details };
}

export function normalizeSecureRedactionPublicationReport(raw) {
  if (!raw || typeof raw !== 'object') {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'SecureRedactionPublicationReport must be an object');
  }
  assertNoUnknownFields(
    raw,
    [
      'schemaVersion', 'schema_version',
      'publicationId', 'publication_id',
      'publishedPath', 'published_path',
      'publishedDigest', 'published_digest',
      'commitState', 'commit_state',
    ],
    'SecureRedactionPublicationReport'
  );

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'SecureRedactionPublicationReport');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED, `Unsupported publication report schemaVersion: '${schemaVersion}'`);
  }

  const publicationIdRaw = readExclusiveAlias(raw, 'publicationId', 'publication_id', true, 'publicationId', 'SecureRedactionPublicationReport');
  const publicationId = assertStrictString(publicationIdRaw, 'publicationId', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_IDENTITY_LEN });

  const publishedPathRaw = readExclusiveAlias(raw, 'publishedPath', 'published_path', true, 'publishedPath', 'SecureRedactionPublicationReport');
  const publishedPath = assertStrictString(publishedPathRaw, 'publishedPath', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_PATH_LEN });

  const publishedDigestRaw = readExclusiveAlias(raw, 'publishedDigest', 'published_digest', true, 'publishedDigest', 'SecureRedactionPublicationReport');
  const publishedDigest = assertStrictString(publishedDigestRaw, 'publishedDigest');
  validateSha256Digest(publishedDigest, 'publishedDigest');

  const commitStateRaw = readExclusiveAlias(raw, 'commitState', 'commit_state', true, 'commitState', 'SecureRedactionPublicationReport');
  const commitState = assertStrictString(commitStateRaw, 'commitState');
  if (!['new_committed', 'new_committed_durability_unconfirmed'].includes(commitState)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `Publication commitState must be a definitive committed state, got '${commitState}'`
    );
  }

  return { schemaVersion, publicationId, publishedPath, publishedDigest, commitState };
}

export function validateUniqueBlockers(blockers) {
  if (!Array.isArray(blockers)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'Blockers must be an array');
  }
  if (blockers.length > REDACTION_LIMITS.MAX_BLOCKERS) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED,
      `Blockers count (${blockers.length}) exceeds maximum limit of ${REDACTION_LIMITS.MAX_BLOCKERS}`
    );
  }
  const seenCodes = new Set();
  return blockers.map(b => {
    const blocker = normalizeRedactionBlocker(b);
    if (seenCodes.has(blocker.code)) {
      // Rationale: Duplicate blocker code is a contract validation error (REDACTION_INVALID_POLICY), not a resource limit error.
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        `Duplicate blocker code found in collection: '${blocker.code}'`
      );
    }
    seenCodes.add(blocker.code);
    return blocker;
  });
}

export function validateUniqueWarnings(warnings) {
  if (!Array.isArray(warnings)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'Warnings must be an array');
  }
  if (warnings.length > REDACTION_LIMITS.MAX_WARNINGS) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED,
      `Warnings count (${warnings.length}) exceeds maximum limit of ${REDACTION_LIMITS.MAX_WARNINGS}`
    );
  }
  const seenCodes = new Set();
  return warnings.map(w => {
    const warning = normalizeRedactionWarning(w);
    if (seenCodes.has(warning.code)) {
      // Rationale: Duplicate warning code is a contract validation error (REDACTION_INVALID_POLICY), not a resource limit error.
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        `Duplicate warning code found in collection: '${warning.code}'`
      );
    }
    seenCodes.add(warning.code);
    return warning;
  });
}

export function validateUniqueVerificationChecks(checks) {
  if (!Array.isArray(checks)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'verificationChecks must be an array');
  }
  if (checks.length > REDACTION_LIMITS.MAX_VERIFICATION_CHECKS) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED,
      `Verification checks count (${checks.length}) exceeds maximum limit of ${REDACTION_LIMITS.MAX_VERIFICATION_CHECKS}`
    );
  }
  const seenCodes = new Set();
  return checks.map(c => {
    const check = normalizeRedactionVerificationCheck(c);
    if (seenCodes.has(check.code)) {
      // Rationale: Duplicate verification check code is a contract validation error (REDACTION_INVALID_POLICY), not a resource limit error.
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        `Duplicate verification check code found in collection: '${check.code}'`
      );
    }
    seenCodes.add(check.code);
    return check;
  });
}

// ---------------------------------------------------------------------------
// 6. Facts & Feature Losses
// ---------------------------------------------------------------------------

/**
 * @typedef {Object} DocumentRedactionFacts
 * @property {string} schemaVersion
 * @property {number} totalPages
 * @property {boolean} isEncrypted
 * @property {boolean} isSigned
 * @property {boolean} hasXfa
 * @property {boolean} hasAcroform
 * @property {boolean} hasAnnotations
 */

/**
 * @typedef {Object} PageRedactionFacts
 * @property {string} schemaVersion
 * @property {number} pageIndex
 * @property {boolean} hasText
 * @property {boolean} hasImages
 * @property {boolean} hasAnnotations
 * @property {0|90|180|270} rotation
 */

/**
 * @typedef {Object} RedactionFeatureLosses
 * @property {string} schemaVersion
 * @property {boolean} textSearchLost
 * @property {boolean} accessibilityLost
 * @property {boolean} annotationsLost
 * @property {boolean} formsLost
 * @property {boolean} vectorGraphicsRasterized
 */

function readMultiAlias(raw, keys, isRequired, fieldName, parentName) {
  const found = keys.filter(k => Object.prototype.hasOwnProperty.call(raw, k));
  if (found.length > 1) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `Ambiguous alias conflict: multiple properties provided for '${fieldName}' in ${parentName}: ${found.join(', ')}`
    );
  }
  if (found.length === 1) {
    return raw[found[0]];
  }
  if (isRequired) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `Required field '${fieldName}' is missing in ${parentName}`
    );
  }
  return undefined;
}

export function normalizeDocumentRedactionFacts(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'DocumentRedactionFacts must be an object');
  }
  assertNoUnknownFields(
    raw,
    [
      'schemaVersion', 'schema_version',
      'totalPages', 'total_pages',
      'encryption', 'isEncrypted', 'is_encrypted',
      'signatureDictionary', 'signatureDictionaryPresent', 'signature_dictionary_present', 'signature_dictionary',
      'signatureField', 'signatureFieldPresent', 'signature_field_present', 'signature_field',
      'byteRange', 'byteRangePresent', 'byte_range_present', 'byte_range',
      'acroForm', 'acro_form', 'acroformPresent', 'acroform_present', 'hasAcroform', 'has_acroform',
      'xfa', 'xfaPresent', 'xfa_present', 'hasXfa', 'has_xfa',
      'annotations', 'annotationsPresent', 'annotations_present', 'hasAnnotations', 'has_annotations',
      'optionalContent', 'optional_content', 'optionalContentPresent', 'optional_content_present',
      'embeddedFiles', 'embedded_files', 'embeddedFilesPresent', 'embedded_files_present',
      'javascript', 'javascriptPresent', 'javascript_present',
      'incrementalRevisions', 'incremental_revisions', 'incrementalRevisionsPresent', 'incremental_revisions_present',
      'structureTree', 'structure_tree',
      'outlines',
      'namesDictionary', 'names_dictionary',
      'documentActions', 'document_actions',
      'complexDocumentResources', 'complex_document_resources',
    ],
    'DocumentRedactionFacts'
  );

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'DocumentRedactionFacts');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED, `Unsupported DocumentRedactionFacts schemaVersion: '${schemaVersion}'`);
  }

  const totalPagesRaw = readExclusiveAlias(raw, 'totalPages', 'total_pages', true, 'totalPages', 'DocumentRedactionFacts');
  const totalPages = assertU32(totalPagesRaw, 'totalPages');

  const encRaw = readMultiAlias(raw, ['encryption', 'isEncrypted', 'is_encrypted'], true, 'encryption', 'DocumentRedactionFacts');
  const sigDictRaw = readMultiAlias(raw, ['signatureDictionary', 'signature_dictionary', 'signatureDictionaryPresent', 'signature_dictionary_present'], true, 'signatureDictionary', 'DocumentRedactionFacts');
  const sigFieldRaw = readMultiAlias(raw, ['signatureField', 'signature_field', 'signatureFieldPresent', 'signature_field_present'], true, 'signatureField', 'DocumentRedactionFacts');
  const byteRangeRaw = readMultiAlias(raw, ['byteRange', 'byte_range', 'byteRangePresent', 'byte_range_present'], true, 'byteRange', 'DocumentRedactionFacts');
  const acroRaw = readMultiAlias(raw, ['acroForm', 'acro_form', 'acroformPresent', 'acroform_present', 'hasAcroform', 'has_acroform'], true, 'acroForm', 'DocumentRedactionFacts');
  const xfaRaw = readMultiAlias(raw, ['xfa', 'xfaPresent', 'xfa_present', 'hasXfa', 'has_xfa'], true, 'xfa', 'DocumentRedactionFacts');
  const annotRaw = readMultiAlias(raw, ['annotations', 'annotationsPresent', 'annotations_present', 'hasAnnotations', 'has_annotations'], true, 'annotations', 'DocumentRedactionFacts');
  const optContentRaw = readMultiAlias(raw, ['optionalContent', 'optional_content', 'optionalContentPresent', 'optional_content_present'], true, 'optionalContent', 'DocumentRedactionFacts');
  const embFilesRaw = readMultiAlias(raw, ['embeddedFiles', 'embedded_files', 'embeddedFilesPresent', 'embedded_files_present'], true, 'embeddedFiles', 'DocumentRedactionFacts');
  const jsRaw = readMultiAlias(raw, ['javascript', 'javascriptPresent', 'javascript_present'], true, 'javascript', 'DocumentRedactionFacts');
  const incrRevRaw = readMultiAlias(raw, ['incrementalRevisions', 'incremental_revisions', 'incrementalRevisionsPresent', 'incremental_revisions_present'], true, 'incrementalRevisions', 'DocumentRedactionFacts');
  const structTreeRaw = readMultiAlias(raw, ['structureTree', 'structure_tree'], true, 'structureTree', 'DocumentRedactionFacts');
  const outlinesRaw = readExclusiveAlias(raw, 'outlines', 'outlines', true, 'outlines', 'DocumentRedactionFacts');
  const namesDictRaw = readMultiAlias(raw, ['namesDictionary', 'names_dictionary'], true, 'namesDictionary', 'DocumentRedactionFacts');
  const docActionsRaw = readMultiAlias(raw, ['documentActions', 'document_actions'], true, 'documentActions', 'DocumentRedactionFacts');
  const cmplxResRaw = readMultiAlias(raw, ['complexDocumentResources', 'complex_document_resources'], true, 'complexDocumentResources', 'DocumentRedactionFacts');

  return {
    schemaVersion,
    totalPages,
    encryption: normalizeInspectionFactState(encRaw, 'encryption'),
    signatureDictionary: normalizeInspectionFactState(sigDictRaw, 'signatureDictionary'),
    signatureField: normalizeInspectionFactState(sigFieldRaw, 'signatureField'),
    byteRange: normalizeInspectionFactState(byteRangeRaw, 'byteRange'),
    acroForm: normalizeInspectionFactState(acroRaw, 'acroForm'),
    xfa: normalizeInspectionFactState(xfaRaw, 'xfa'),
    annotations: normalizeInspectionFactState(annotRaw, 'annotations'),
    optionalContent: normalizeInspectionFactState(optContentRaw, 'optionalContent'),
    embeddedFiles: normalizeInspectionFactState(embFilesRaw, 'embeddedFiles'),
    javascript: normalizeInspectionFactState(jsRaw, 'javascript'),
    incrementalRevisions: normalizeInspectionFactState(incrRevRaw, 'incrementalRevisions'),
    structureTree: normalizeInspectionFactState(structTreeRaw, 'structureTree'),
    outlines: normalizeInspectionFactState(outlinesRaw, 'outlines'),
    namesDictionary: normalizeInspectionFactState(namesDictRaw, 'namesDictionary'),
    documentActions: normalizeInspectionFactState(docActionsRaw, 'documentActions'),
    complexDocumentResources: normalizeInspectionFactState(cmplxResRaw, 'complexDocumentResources'),
  };
}

export function normalizePageRedactionFacts(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'PageRedactionFacts must be an object');
  }
  assertNoUnknownFields(
    raw,
    [
      'schemaVersion', 'schema_version',
      'pageIndex', 'page_index',
      'text', 'textPresent', 'text_present', 'hasText', 'has_text',
      'images', 'imagePresent', 'image_present', 'hasImages', 'has_images',
      'vectorPaths', 'vector_paths', 'vectorPathPresent', 'vector_path_present',
      'formXobjects', 'form_xobjects', 'formXobjectPresent', 'form_xobject_present',
      'annotations', 'annotationPresent', 'annotation_present', 'hasAnnotations', 'has_annotations',
      'linkAnnotations', 'link_annotations',
      'widgets', 'widgetPresent', 'widget_present',
      'transparency', 'transparencyPresent', 'transparency_present',
      'optionalContent', 'optional_content', 'optionalContentPresent', 'optional_content_present',
      'complexResources', 'complex_resources', 'complexResourcesPresent', 'complex_resources_present',
      'structureAssociation', 'structure_association',
      'pageActions', 'page_actions',
      'spotColors', 'spot_colors',
      'complexColorSpaces', 'complex_color_spaces',
      'geometryProfile', 'geometry_profile',
      'rotation',
    ],
    'PageRedactionFacts'
  );

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'PageRedactionFacts');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED, `Unsupported PageRedactionFacts schemaVersion: '${schemaVersion}'`);
  }

  const pageIndexRaw = readExclusiveAlias(raw, 'pageIndex', 'page_index', true, 'pageIndex', 'PageRedactionFacts');
  const pageIndex = assertU32(pageIndexRaw, 'pageIndex');

  const rotation = assertStrictNumber(raw.rotation, 'rotation', { integer: true });
  if (![0, 90, 180, 270].includes(rotation)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_PAGE_GEOMETRY_MISMATCH, `Invalid page rotation: ${rotation}`);
  }

  const textRaw = readMultiAlias(raw, ['text', 'textPresent', 'text_present', 'hasText', 'has_text'], true, 'text', 'PageRedactionFacts');
  const imagesRaw = readMultiAlias(raw, ['images', 'imagePresent', 'image_present', 'hasImages', 'has_images'], true, 'images', 'PageRedactionFacts');
  const vecRaw = readMultiAlias(raw, ['vectorPaths', 'vector_paths', 'vectorPathPresent', 'vector_path_present'], true, 'vectorPaths', 'PageRedactionFacts');
  const formXRaw = readMultiAlias(raw, ['formXobjects', 'form_xobjects', 'formXobjectPresent', 'form_xobject_present'], true, 'formXobjects', 'PageRedactionFacts');
  const annotRaw = readMultiAlias(raw, ['annotations', 'annotationPresent', 'annotation_present', 'hasAnnotations', 'has_annotations'], true, 'annotations', 'PageRedactionFacts');
  const linkAnnotRaw = readMultiAlias(raw, ['linkAnnotations', 'link_annotations'], true, 'linkAnnotations', 'PageRedactionFacts');
  const widgetRaw = readMultiAlias(raw, ['widgets', 'widgetPresent', 'widget_present'], true, 'widgets', 'PageRedactionFacts');
  const transRaw = readMultiAlias(raw, ['transparency', 'transparencyPresent', 'transparency_present'], true, 'transparency', 'PageRedactionFacts');
  const optRaw = readMultiAlias(raw, ['optionalContent', 'optional_content', 'optionalContentPresent', 'optional_content_present'], true, 'optionalContent', 'PageRedactionFacts');
  const cmplxRaw = readMultiAlias(raw, ['complexResources', 'complex_resources', 'complexResourcesPresent', 'complex_resources_present'], true, 'complexResources', 'PageRedactionFacts');
  const structAssocRaw = readMultiAlias(raw, ['structureAssociation', 'structure_association'], true, 'structureAssociation', 'PageRedactionFacts');
  const pageActionsRaw = readMultiAlias(raw, ['pageActions', 'page_actions'], true, 'pageActions', 'PageRedactionFacts');
  const spotColorsRaw = readMultiAlias(raw, ['spotColors', 'spot_colors'], true, 'spotColors', 'PageRedactionFacts');
  const cmplxColorRaw = readMultiAlias(raw, ['complexColorSpaces', 'complex_color_spaces'], true, 'complexColorSpaces', 'PageRedactionFacts');
  const geomProfRaw = readExclusiveAlias(raw, 'geometryProfile', 'geometry_profile', true, 'geometryProfile', 'PageRedactionFacts');
  const geomProfStr = assertStrictString(geomProfRaw, 'geometryProfile');
  if (geomProfStr !== 'standard_effective_box_v1') {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `Unsupported geometryProfile: '${geomProfStr}'. Expected 'standard_effective_box_v1'`
    );
  }

  return {
    schemaVersion,
    pageIndex,
    text: normalizeInspectionFactState(textRaw, 'text'),
    images: normalizeInspectionFactState(imagesRaw, 'images'),
    vectorPaths: normalizeInspectionFactState(vecRaw, 'vectorPaths'),
    formXobjects: normalizeInspectionFactState(formXRaw, 'formXobjects'),
    annotations: normalizeInspectionFactState(annotRaw, 'annotations'),
    linkAnnotations: normalizeInspectionFactState(linkAnnotRaw, 'linkAnnotations'),
    widgets: normalizeInspectionFactState(widgetRaw, 'widgets'),
    transparency: normalizeInspectionFactState(transRaw, 'transparency'),
    optionalContent: normalizeInspectionFactState(optRaw, 'optionalContent'),
    complexResources: normalizeInspectionFactState(cmplxRaw, 'complexResources'),
    structureAssociation: normalizeInspectionFactState(structAssocRaw, 'structureAssociation'),
    pageActions: normalizeInspectionFactState(pageActionsRaw, 'pageActions'),
    spotColors: normalizeInspectionFactState(spotColorsRaw, 'spotColors'),
    complexColorSpaces: normalizeInspectionFactState(cmplxColorRaw, 'complexColorSpaces'),
    geometryProfile: geomProfStr,
    rotation,
  };
}

export function normalizeRedactionFeatureLosses(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'RedactionFeatureLosses must be an object');
  }
  assertNoUnknownFields(
    raw,
    [
      'schemaVersion', 'schema_version',
      'textSelectionLost', 'text_selection_lost', 'textSearchLost', 'text_search_lost',
      'searchabilityLost', 'searchability_lost',
      'accessibilityTagsLost', 'accessibility_tags_lost', 'accessibilityLost', 'accessibility_lost',
      'linksLost', 'links_lost',
      'annotationsLost', 'annotations_lost',
      'formsLost', 'forms_lost',
      'vectorQualityLost', 'vector_quality_lost', 'vectorGraphicsRasterized', 'vector_graphics_rasterized',
      'colorSpaceFidelityRisk', 'color_space_fidelity_risk',
      'spotColorsLost', 'spot_colors_lost',
      'layersLost', 'layers_lost',
      'structureAssociationLost', 'structure_association_lost',
      'pageActionsLost', 'page_actions_lost',
    ],
    'RedactionFeatureLosses'
  );

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'RedactionFeatureLosses');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED, `Unsupported RedactionFeatureLosses schemaVersion: '${schemaVersion}'`);
  }

  const searchabilityRaw = readExclusiveAlias(raw, 'searchabilityLost', 'searchability_lost', true, 'searchabilityLost', 'RedactionFeatureLosses');
  const linksRaw = readExclusiveAlias(raw, 'linksLost', 'links_lost', true, 'linksLost', 'RedactionFeatureLosses');
  const colorRiskRaw = readExclusiveAlias(raw, 'colorSpaceFidelityRisk', 'color_space_fidelity_risk', true, 'colorSpaceFidelityRisk', 'RedactionFeatureLosses');
  const spotRaw = readExclusiveAlias(raw, 'spotColorsLost', 'spot_colors_lost', true, 'spotColorsLost', 'RedactionFeatureLosses');
  const layersRaw = readExclusiveAlias(raw, 'layersLost', 'layers_lost', true, 'layersLost', 'RedactionFeatureLosses');
  const structAssocRaw = readMultiAlias(raw, ['structureAssociationLost', 'structure_association_lost'], true, 'structureAssociationLost', 'RedactionFeatureLosses');
  const pageActionsRaw = readMultiAlias(raw, ['pageActionsLost', 'page_actions_lost'], true, 'pageActionsLost', 'RedactionFeatureLosses');

  const textSel = assertStrictBoolean(readMultiAlias(raw, ['textSelectionLost', 'text_selection_lost', 'textSearchLost', 'text_search_lost'], true, 'textSelectionLost', 'RedactionFeatureLosses'), 'textSelectionLost');
  const a11yTags = assertStrictBoolean(readMultiAlias(raw, ['accessibilityTagsLost', 'accessibility_tags_lost', 'accessibilityLost', 'accessibility_lost'], true, 'accessibilityTagsLost', 'RedactionFeatureLosses'), 'accessibilityTagsLost');

  return {
    schemaVersion,
    textSelectionLost: textSel,
    searchabilityLost: assertStrictBoolean(searchabilityRaw, 'searchabilityLost'),
    accessibilityTagsLost: a11yTags,
    linksLost: assertStrictBoolean(linksRaw, 'linksLost'),
    annotationsLost: assertStrictBoolean(readExclusiveAlias(raw, 'annotationsLost', 'annotations_lost', true, 'annotationsLost', 'RedactionFeatureLosses'), 'annotationsLost'),
    formsLost: assertStrictBoolean(readExclusiveAlias(raw, 'formsLost', 'forms_lost', true, 'formsLost', 'RedactionFeatureLosses'), 'formsLost'),
    vectorQualityLost: assertStrictBoolean(readMultiAlias(raw, ['vectorQualityLost', 'vector_quality_lost', 'vectorGraphicsRasterized', 'vector_graphics_rasterized'], true, 'vectorQualityLost', 'RedactionFeatureLosses'), 'vectorQualityLost'),
    colorSpaceFidelityRisk: assertStrictBoolean(colorRiskRaw, 'colorSpaceFidelityRisk'),
    spotColorsLost: assertStrictBoolean(spotRaw, 'spotColorsLost'),
    layersLost: assertStrictBoolean(layersRaw, 'layersLost'),
    structureAssociationLost: assertStrictBoolean(structAssocRaw, 'structureAssociationLost'),
    pageActionsLost: assertStrictBoolean(pageActionsRaw, 'pageActionsLost'),
  };
}

export function derivePageFeatureLosses(facts, strategy) {
  if (strategy === 'import_unaffected_page') {
    return {
      schemaVersion: REDACTION_SCHEMA_VERSION,
      textSelectionLost: false,
      searchabilityLost: false,
      accessibilityTagsLost: false,
      linksLost: false,
      annotationsLost: false,
      formsLost: false,
      vectorQualityLost: false,
      colorSpaceFidelityRisk: false,
      spotColorsLost: false,
      layersLost: false,
      structureAssociationLost: false,
      pageActionsLost: false,
    };
  }
  return {
    schemaVersion: REDACTION_SCHEMA_VERSION,
    textSelectionLost: facts.text === 'present',
    searchabilityLost: facts.text === 'present',
    accessibilityTagsLost: facts.structureAssociation === 'present',
    linksLost: facts.linkAnnotations === 'present',
    annotationsLost: facts.annotations === 'present',
    formsLost: facts.widgets === 'present',
    vectorQualityLost: facts.vectorPaths === 'present' || facts.formXobjects === 'present',
    colorSpaceFidelityRisk: facts.complexResources === 'present' || facts.transparency === 'present' || facts.complexColorSpaces === 'present',
    spotColorsLost: facts.spotColors === 'present',
    layersLost: facts.optionalContent === 'present',
    structureAssociationLost: facts.structureAssociation === 'present',
    pageActionsLost: facts.pageActions === 'present',
  };
}

export function aggregateDocumentFeatureLosses(pagePlans) {
  let textSelectionLost = false;
  let searchabilityLost = false;
  let accessibilityTagsLost = false;
  let linksLost = false;
  let annotationsLost = false;
  let formsLost = false;
  let vectorQualityLost = false;
  let colorSpaceFidelityRisk = false;
  let spotColorsLost = false;
  let layersLost = false;
  let structureAssociationLost = false;
  let pageActionsLost = false;

  for (const pagePlan of pagePlans) {
    const l = pagePlan.featureLosses;
    if (l.textSelectionLost) textSelectionLost = true;
    if (l.searchabilityLost) searchabilityLost = true;
    if (l.accessibilityTagsLost) accessibilityTagsLost = true;
    if (l.linksLost) linksLost = true;
    if (l.annotationsLost) annotationsLost = true;
    if (l.formsLost) formsLost = true;
    if (l.vectorQualityLost) vectorQualityLost = true;
    if (l.colorSpaceFidelityRisk) colorSpaceFidelityRisk = true;
    if (l.spotColorsLost) spotColorsLost = true;
    if (l.layersLost) layersLost = true;
    if (l.structureAssociationLost) structureAssociationLost = true;
    if (l.pageActionsLost) pageActionsLost = true;
  }

  return {
    schemaVersion: REDACTION_SCHEMA_VERSION,
    textSelectionLost,
    searchabilityLost,
    accessibilityTagsLost,
    linksLost,
    annotationsLost,
    formsLost,
    vectorQualityLost,
    colorSpaceFidelityRisk,
    spotColorsLost,
    layersLost,
    structureAssociationLost,
    pageActionsLost,
  };
}

export function deriveAcknowledgementRequirements(pagePlans, documentLosses, documentFacts) {
  const rasterizedPagesRequired = pagePlans.some(p => p.strategy === 'rasterize_page');
  const textSearchLossRequired = documentLosses.searchabilityLost;
  const accessibilityLossRequired = documentLosses.accessibilityTagsLost || documentLosses.structureAssociationLost;
  const annotationLossRequired = documentLosses.annotationsLost || documentLosses.linksLost;
  const formLossRequired = documentLosses.formsLost;
  const signatureInvalidationRequired = documentFacts.signatureDictionary === 'present' || documentFacts.signatureField === 'present' || documentFacts.signatureDictionary === 'unknown' || documentFacts.signatureField === 'unknown';
  const encryptionChangeRequired = documentFacts.encryption === 'present' || documentFacts.encryption === 'unknown';

  return {
    schemaVersion: REDACTION_SCHEMA_VERSION,
    rasterizedPagesRequired,
    textSearchLossRequired,
    accessibilityLossRequired,
    annotationLossRequired,
    formLossRequired,
    signatureInvalidationRequired,
    encryptionChangeRequired,
  };
}

export function evaluatePageBlockers(pageFacts) {
  const pageBlockers = [];
  if (
    pageFacts.complexResources === 'unknown' ||
    pageFacts.optionalContent === 'unknown' ||
    pageFacts.structureAssociation === 'unknown' ||
    pageFacts.pageActions === 'unknown' ||
    pageFacts.complexColorSpaces === 'unknown' ||
    pageFacts.spotColors === 'unknown' ||
    pageFacts.transparency === 'unknown' ||
    pageFacts.widgets === 'unknown' ||
    pageFacts.linkAnnotations === 'unknown' ||
    pageFacts.formXobjects === 'unknown' ||
    pageFacts.vectorPaths === 'unknown'
  ) {
    pageBlockers.push({
      schemaVersion: REDACTION_SCHEMA_VERSION,
      code: REDACTION_ERROR_CODES.REDACTION_BLOCKED_DOCUMENT_FEATURE,
      message: `Page ${pageFacts.pageIndex} contains critical unknown inspection fact state`,
      details: null,
    });
  }
  return pageBlockers;
}

/**
 * Truncate UTF-8 string to maximum byte limit without producing invalid surrogates
 * @param {string} str
 * @param {number} limitBytes
 * @returns {string}
 */
export function truncateUtf8ToByteLimit(str, limitBytes) {
  if (typeof str !== 'string') return '';
  const encoder = new TextEncoder();
  const bytes = encoder.encode(str);
  if (bytes.length <= limitBytes) {
    return str;
  }
  let end = limitBytes;
  while (end > 0 && (bytes[end] & 0xc0) === 0x80) {
    end--;
  }
  return new TextDecoder('utf-8', { fatal: false }).decode(bytes.subarray(0, end));
}

function aggregateDetailsString(d1, d2) {
  const parts = [];
  if (d1) {
    for (const part of d1.split('; ')) {
      const trimmed = part.trim();
      if (trimmed.length > 0 && !parts.includes(trimmed)) {
        parts.push(trimmed);
      }
    }
  }
  if (d2) {
    for (const part of d2.split('; ')) {
      const trimmed = part.trim();
      if (trimmed.length > 0 && !parts.includes(trimmed)) {
        parts.push(trimmed);
      }
    }
  }
  if (parts.length === 0) {
    return null;
  }
  parts.sort(compareCanonicalUtf8Strings);

  let result = '';
  for (const part of parts) {
    if (result.length > 0) {
      if (getUtf8ByteLength(result) + 2 > REDACTION_LIMITS.MAX_DETAILS_LEN) {
        break;
      }
      result += '; ';
    }
    const currentLen = getUtf8ByteLength(result);
    const partLen = getUtf8ByteLength(part);
    if (currentLen + partLen <= REDACTION_LIMITS.MAX_DETAILS_LEN) {
      result += part;
    } else {
      const remainingBudget = REDACTION_LIMITS.MAX_DETAILS_LEN - currentLen;
      const truncatedPart = truncateUtf8ToByteLimit(part, remainingBudget);
      const trimmed = truncatedPart.replace(/\s+$/g, '');
      if (trimmed.length > 0) {
        result += trimmed;
      }
      break;
    }
  }

  while (result.endsWith(';') || result.endsWith(' ')) {
    if (result.endsWith('; ')) {
      result = result.slice(0, -2);
    } else {
      result = result.slice(0, -1);
    }
  }

  return result.length === 0 ? null : result;
}

export function pushAggregatedBlocker(blockers, blocker) {
  const existing = blockers.find(b => b.code === blocker.code);
  if (existing) {
    existing.details = aggregateDetailsString(existing.details, blocker.details);
  } else {
    const b = { ...blocker };
    b.details = aggregateDetailsString(null, b.details);
    blockers.push(b);
  }
}

export function pushAggregatedWarning(warnings, warning) {
  const existing = warnings.find(w => w.code === warning.code);
  if (existing) {
    existing.details = aggregateDetailsString(existing.details, warning.details);
  } else {
    const w = { ...warning };
    w.details = aggregateDetailsString(null, w.details);
    warnings.push(w);
  }
}

export function hasMergedSourceIdOverflow(regions) {
  const n = regions.length;
  for (let i = 0; i < n; i++) {
    for (let j = i + 1; j < n; j++) {
      const r1 = regions[i].appliedPdfPointRect;
      const r2 = regions[j].appliedPdfPointRect;

      const r1Right = r1.left + r1.width;
      const r1Top = r1.bottom + r1.height;
      const r2Right = r2.left + r2.width;
      const r2Top = r2.bottom + r2.height;

      const xGap = r2.left > r1Right ? r2.left - r1Right : (r1.left > r2Right ? r1.left - r2Right : 0.0);
      const yGap = r2.bottom > r1Top ? r2.bottom - r1Top : (r1.bottom > r2Top ? r1.bottom - r2Top : 0.0);

      const hasXOverlap = r1.left < r2Right && r2.left < r1Right;
      const hasYOverlap = r1.bottom < r2Top && r2.bottom < r1Top;

      const isXTouch = Math.abs(r1.left - r2Right) <= GEOMETRY_EPSILON || Math.abs(r1Right - r2.left) <= GEOMETRY_EPSILON;
      const isYTouch = Math.abs(r1.bottom - r2Top) <= GEOMETRY_EPSILON || Math.abs(r1Top - r2.bottom) <= GEOMETRY_EPSILON;

      const isTrueOverlap = hasXOverlap && hasYOverlap;
      const isEdgeTouch = (isXTouch && hasYOverlap) || (isYTouch && hasXOverlap);
      const isCornerTouch = isXTouch && isYTouch;

      let shouldMerge = false;
      if (xGap > 0.0 || yGap > 0.0) {
        shouldMerge = false;
      } else if (isTrueOverlap) {
        shouldMerge = true;
      } else if (isEdgeTouch) {
        shouldMerge = CANONICAL_MERGE_PROFILE.mergeEdgeTouching;
      } else if (isCornerTouch) {
        shouldMerge = CANONICAL_MERGE_PROFILE.mergeCornerTouching;
      }

      if (shouldMerge) {
        const combined = Array.from(new Set([...regions[i].sourceRegionIds, ...regions[j].sourceRegionIds]));
        if (combined.length > REDACTION_LIMITS.MAX_SOURCE_REGION_IDS) {
          return true;
        }
      }
    }
  }
  return false;
}

// ---------------------------------------------------------------------------
// 7. Policy Helper for Document Feature Blockers
// ---------------------------------------------------------------------------

export function evaluateDocumentBlockers(facts, policy, _ack, pagePlans = null) {
  const blockers = [];
  if (facts.totalPages > REDACTION_LIMITS.MAX_PAGE_PLANS) {
    pushAggregatedBlocker(blockers, {
      schemaVersion: REDACTION_SCHEMA_VERSION,
      code: REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED,
      message: `Total page count (${facts.totalPages}) exceeds maximum page plan limit of ${REDACTION_LIMITS.MAX_PAGE_PLANS}`,
      details: null,
    });
  }
  if (pagePlans && Array.isArray(pagePlans)) {
    let totalEstimatedPixels = 0;
    for (const pagePlan of pagePlans) {
      if (pagePlan.strategy === 'rasterize_page') {
        const widthPx = Math.ceil((pagePlan.geometry.effectiveBox.width / 72.0) * policy.rasterDpi);
        const heightPx = Math.ceil((pagePlan.geometry.effectiveBox.height / 72.0) * policy.rasterDpi);
        const pagePixels = widthPx * heightPx;
        if (pagePixels > REDACTION_LIMITS.MAX_PIXELS_PER_PAGE) {
          pushAggregatedBlocker(blockers, {
            schemaVersion: REDACTION_SCHEMA_VERSION,
            code: REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED,
            message: `Page ${pagePlan.pageIndex} estimated pixel count (${pagePixels}) exceeds limit of ${REDACTION_LIMITS.MAX_PIXELS_PER_PAGE}`,
            details: null,
          });
        }
        totalEstimatedPixels += pagePixels;
      }
    }
    if (totalEstimatedPixels > REDACTION_LIMITS.MAX_TOTAL_ESTIMATED_PIXELS) {
      pushAggregatedBlocker(blockers, {
        schemaVersion: REDACTION_SCHEMA_VERSION,
        code: REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED,
        message: `Total estimated pixels (${totalEstimatedPixels}) exceeds limit of ${REDACTION_LIMITS.MAX_TOTAL_ESTIMATED_PIXELS}`,
        details: null,
      });
    }
  }
  const isEncrypted = facts.encryption === 'present' || facts.encryption === 'unknown' || facts.isEncrypted === true || facts.is_encrypted === true;
  if (isEncrypted && policy.encryptionPolicy === 'block') {
    pushAggregatedBlocker(blockers, {
      schemaVersion: REDACTION_SCHEMA_VERSION,
      code: REDACTION_ERROR_CODES.REDACTION_ENCRYPTED_INPUT_UNSUPPORTED,
      message: 'Encrypted inputs are unsupported in current redaction profile',
      details: 'Encryption change acknowledgement cannot bypass this policy blocker',
    });
  }
  const isSigned = facts.signatureDictionary === 'present' || facts.signatureDictionary === 'unknown' ||
                   facts.signatureField === 'present' || facts.signatureField === 'unknown' ||
                   facts.byteRange === 'present' || facts.byteRange === 'unknown' ||
                   facts.isSigned === true || facts.is_signed === true ||
                   facts.signatureDictionaryPresent === true || facts.signatureFieldPresent === true || facts.byteRangePresent === true;
  if (isSigned && policy.signaturePolicy === 'block') {
    pushAggregatedBlocker(blockers, {
      schemaVersion: REDACTION_SCHEMA_VERSION,
      code: REDACTION_ERROR_CODES.REDACTION_SIGNED_INPUT_UNSUPPORTED,
      message: 'Signed inputs are unsupported in current redaction profile',
      details: 'Signature invalidation acknowledgement cannot bypass this policy blocker',
    });
  }
  const isXfa = facts.xfa === 'present' || facts.xfa === 'unknown' || facts.xfaPresent === true || facts.hasXfa === true || facts.has_xfa === true;
  if (isXfa) {
    pushAggregatedBlocker(blockers, {
      schemaVersion: REDACTION_SCHEMA_VERSION,
      code: REDACTION_ERROR_CODES.REDACTION_XFA_INPUT_UNSUPPORTED,
      message: 'XFA dynamic forms are unsupported in current redaction profile',
      details: null,
    });
  }
  const isAcroform = facts.acroForm === 'present' || facts.acroForm === 'unknown' || facts.acroformPresent === true || facts.hasAcroform === true || facts.has_acroform === true;
  if (isAcroform && policy.interactiveContentPolicy === 'block') {
    pushAggregatedBlocker(blockers, {
      schemaVersion: REDACTION_SCHEMA_VERSION,
      code: REDACTION_ERROR_CODES.REDACTION_BLOCKED_DOCUMENT_FEATURE,
      message: 'Interactive AcroForm features are blocked under conservative policy',
      details: null,
    });
  }
  if (facts.complexDocumentResources === 'present' || facts.complexDocumentResources === 'unknown') {
    pushAggregatedBlocker(blockers, {
      schemaVersion: REDACTION_SCHEMA_VERSION,
      code: REDACTION_ERROR_CODES.REDACTION_BLOCKED_DOCUMENT_FEATURE,
      message: 'Complex document resources present or unknown',
      details: null,
    });
  }
  if (facts.javascript === 'present' || facts.javascript === 'unknown') {
    pushAggregatedBlocker(blockers, {
      schemaVersion: REDACTION_SCHEMA_VERSION,
      code: REDACTION_ERROR_CODES.REDACTION_BLOCKED_DOCUMENT_FEATURE,
      message: 'Document JavaScript present or unknown',
      details: null,
    });
  }
  if (facts.documentActions === 'present' || facts.documentActions === 'unknown') {
    pushAggregatedBlocker(blockers, {
      schemaVersion: REDACTION_SCHEMA_VERSION,
      code: REDACTION_ERROR_CODES.REDACTION_BLOCKED_DOCUMENT_FEATURE,
      message: 'Document actions present or unknown',
      details: null,
    });
  }
  if (facts.embeddedFiles === 'present' || facts.embeddedFiles === 'unknown') {
    pushAggregatedBlocker(blockers, {
      schemaVersion: REDACTION_SCHEMA_VERSION,
      code: REDACTION_ERROR_CODES.REDACTION_BLOCKED_DOCUMENT_FEATURE,
      message: 'Embedded files present or unknown',
      details: null,
    });
  }
  if (facts.namesDictionary === 'present' || facts.namesDictionary === 'unknown') {
    pushAggregatedBlocker(blockers, {
      schemaVersion: REDACTION_SCHEMA_VERSION,
      code: REDACTION_ERROR_CODES.REDACTION_BLOCKED_DOCUMENT_FEATURE,
      message: 'Names dictionary present or unknown',
      details: null,
    });
  }
  return blockers;
}

// ---------------------------------------------------------------------------
// 8. Page Plan & Secure Redaction Plan
// ---------------------------------------------------------------------------

function readRegionsAlias(raw) {
  const keys = ['canonicalRegions', 'canonical_regions', 'regions'].filter(k => Object.prototype.hasOwnProperty.call(raw, k));
  if (keys.length > 1) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `Ambiguous alias conflict: multiple region properties provided in PageRedactionPlan: ${keys.join(', ')}`
    );
  }
  if (keys.length === 1) return raw[keys[0]];
  throw new RedactionError(
    REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
    "Required field 'canonicalRegions' is missing in PageRedactionPlan"
  );
}

export function normalizePageRedactionPlan(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'PageRedactionPlan must be an object');
  }
  assertNoUnknownFields(
    raw,
    [
      'schemaVersion', 'schema_version',
      'pageIndex', 'page_index',
      'strategy',
      'geometry',
      'geometryDigest', 'geometry_digest',
      'canonicalRegions', 'canonical_regions', 'regions',
      'facts',
      'featureLosses', 'feature_losses',
      'blockers',
      'warnings',
      'executable',
    ],
    'PageRedactionPlan'
  );

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'PageRedactionPlan');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED, `Unsupported PageRedactionPlan schemaVersion: '${schemaVersion}'`);
  }

  const pageIndexRaw = readExclusiveAlias(raw, 'pageIndex', 'page_index', true, 'pageIndex', 'PageRedactionPlan');
  const pageIndex = assertU32(pageIndexRaw, 'pageIndex');

  const strategy = assertStrictString(raw.strategy, 'strategy');
  if (strategy !== 'rasterize_page' && strategy !== 'import_unaffected_page') {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, `Unsupported page redaction strategy: '${strategy}'`);
  }

  if (!raw.geometry || typeof raw.geometry !== 'object' || Array.isArray(raw.geometry)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'geometry must be a valid object in PageRedactionPlan');
  }
  const geometry = normalizeCanonicalPageGeometry(raw.geometry);
  if (geometry.pageIndex !== pageIndex) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_PAGE_GEOMETRY_MISMATCH,
      `Page plan pageIndex (${pageIndex}) mismatch with geometry pageIndex (${geometry.pageIndex})`
    );
  }

  const geomDigestRaw = readExclusiveAlias(raw, 'geometryDigest', 'geometry_digest', true, 'geometryDigest', 'PageRedactionPlan');
  const geometryDigest = assertStrictString(geomDigestRaw, 'geometryDigest');
  validateSha256Digest(geometryDigest, 'geometryDigest');

  const rawRegions = readRegionsAlias(raw);
  if (!Array.isArray(rawRegions)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_REGION, 'PageRedactionPlan canonicalRegions must be an array');
  }
  if (rawRegions.length > REDACTION_LIMITS.MAX_CANONICAL_REGIONS_PER_PAGE) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED, 'Page canonicalRegions count exceeds limit');
  }
  const canonicalRegions = rawRegions.map(r => normalizeCanonicalRedactionRegion(r, geometry, undefined));
  let prevRegion = null;
  for (const region of canonicalRegions) {
    if (region.pageIndex !== pageIndex) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_PAGE_GEOMETRY_MISMATCH,
        `Canonical region pageIndex (${region.pageIndex}) mismatch with parent page plan pageIndex (${pageIndex})`
      );
    }
    if (prevRegion !== null) {
      if (compareCanonicalRegions(prevRegion, region) > 0) {
        throw new RedactionError(
          REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
          'Canonical regions must be in canonical sort order (pageIndex, bottom, left, regionId)'
        );
      }
    }
    prevRegion = region;
  }

  const facts = normalizePageRedactionFacts(raw.facts);
  if (facts.pageIndex !== pageIndex) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_PAGE_GEOMETRY_MISMATCH,
      `Page plan pageIndex (${pageIndex}) mismatch with page facts pageIndex (${facts.pageIndex})`
    );
  }
  if (facts.rotation !== geometry.rotation) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_PAGE_GEOMETRY_MISMATCH,
      `Page facts rotation (${facts.rotation}) mismatch with page geometry rotation (${geometry.rotation})`
    );
  }

  if (strategy === 'rasterize_page' && canonicalRegions.length === 0) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'Strategy rasterize_page requires at least 1 canonical region'
    );
  }
  if (strategy === 'import_unaffected_page' && canonicalRegions.length > 0) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'Strategy import_unaffected_page must have exactly 0 canonical regions'
    );
  }

  const featureLossesRaw = readExclusiveAlias(raw, 'featureLosses', 'feature_losses', true, 'featureLosses', 'PageRedactionPlan');
  const featureLosses = normalizeRedactionFeatureLosses(featureLossesRaw);

  const derivedPageLosses = derivePageFeatureLosses(facts, strategy);
  for (const k of Object.keys(derivedPageLosses)) {
    if (k !== 'schemaVersion' && featureLosses[k] !== derivedPageLosses[k]) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        `Page plan featureLosses.${k} (${featureLosses[k]}) does not match derived value (${derivedPageLosses[k]})`
      );
    }
  }

  const rawBlockers = readExclusiveAlias(raw, 'blockers', 'blockers', true, 'blockers', 'PageRedactionPlan');
  const blockers = validateUniqueBlockers(rawBlockers);

  const expectedPageBlockers = evaluatePageBlockers(facts);
  const hasSourceIdOverflow = hasMergedSourceIdOverflow(canonicalRegions);
  if (canonicalRegions.length > REDACTION_LIMITS.MAX_CANONICAL_REGIONS_PER_PAGE || hasSourceIdOverflow) {
    pushAggregatedBlocker(expectedPageBlockers, {
      schemaVersion: REDACTION_SCHEMA_VERSION,
      code: REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED,
      message: hasSourceIdOverflow
        ? `Page ${pageIndex} merged canonical region source_region_ids count would exceed limit of ${REDACTION_LIMITS.MAX_SOURCE_REGION_IDS}`
        : `Page ${pageIndex} merged canonical regions count (${canonicalRegions.length}) exceeds maximum limit of ${REDACTION_LIMITS.MAX_CANONICAL_REGIONS_PER_PAGE}`,
      details: null,
    });
  }
  if (JSON.stringify(blockers) !== JSON.stringify(expectedPageBlockers)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `Page plan blockers mismatch derived page blockers`
    );
  }

  const rawWarnings = readExclusiveAlias(raw, 'warnings', 'warnings', true, 'warnings', 'PageRedactionPlan');
  const warnings = validateUniqueWarnings(rawWarnings);

  const execRaw = readExclusiveAlias(raw, 'executable', 'executable', true, 'executable', 'PageRedactionPlan');
  const executable = assertStrictBoolean(execRaw, 'executable');

  const expectedPageExecutable = (blockers.length === 0);
  if (executable !== expectedPageExecutable) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `Page plan executable (${executable}) does not match expected (${expectedPageExecutable})`
    );
  }

  return {
    schemaVersion,
    pageIndex,
    strategy,
    geometry,
    geometryDigest,
    canonicalRegions,
    facts,
    featureLosses,
    blockers,
    warnings,
    executable,
  };
}

/**
 * Validates cross-field plan invariants on fully normalized camelCase plan object.
 * @param {Record<string, any>} plan
 */
function assertNormalizedSecureRedactionPlan(plan) {
  if (plan.pagePlans.length !== plan.facts.totalPages) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `pagePlans count (${plan.pagePlans.length}) must strictly equal document totalPages (${plan.facts.totalPages})`
    );
  }

  for (let i = 0; i < plan.facts.totalPages; i++) {
    const pagePlan = plan.pagePlans[i];
    if (!pagePlan || pagePlan.pageIndex !== i) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        `pagePlans must contain strictly ordered pageIndex sequence 0..totalPages-1. Index mismatch at position ${i}`
      );
    }
  }

  const derivedDocLosses = aggregateDocumentFeatureLosses(plan.pagePlans);
  for (const k of Object.keys(derivedDocLosses)) {
    if (k !== 'schemaVersion' && plan.losses[k] !== derivedDocLosses[k]) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        `Document losses.${k} (${plan.losses[k]}) does not match aggregated page losses (${derivedDocLosses[k]})`
      );
    }
  }

  const derivedAckReqs = deriveAcknowledgementRequirements(plan.pagePlans, plan.losses, plan.facts);
  for (const k of Object.keys(derivedAckReqs)) {
    if (k !== 'schemaVersion' && plan.acknowledgementRequirements[k] !== derivedAckReqs[k]) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        `Document acknowledgementRequirements.${k} (${plan.acknowledgementRequirements[k]}) does not match derived value (${derivedAckReqs[k]})`
      );
    }
  }

  const expectedDocBlockers = evaluateDocumentBlockers(plan.facts, plan.policy, plan.acknowledgements, plan.pagePlans);
  const expectedTopLevelBlockers = [...expectedDocBlockers];
  const expectedTopLevelWarnings = [];
  for (const pagePlan of plan.pagePlans) {
    for (const b of pagePlan.blockers) {
      pushAggregatedBlocker(expectedTopLevelBlockers, b);
    }
    for (const w of pagePlan.warnings) {
      pushAggregatedWarning(expectedTopLevelWarnings, w);
    }
  }
  if (JSON.stringify(plan.blockers) !== JSON.stringify(expectedTopLevelBlockers)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'Top-level blockers mismatch expected aggregated blockers'
    );
  }
  if (JSON.stringify(plan.warnings) !== JSON.stringify(expectedTopLevelWarnings)) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'Top-level warnings mismatch expected aggregated warnings'
    );
  }

  const flattenedRegions = plan.pagePlans.flatMap(p => p.canonicalRegions);
  if (plan.canonicalRegions.length !== flattenedRegions.length) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `Top-level canonicalRegions count (${plan.canonicalRegions.length}) does not match flattened page plan canonicalRegions count (${flattenedRegions.length})`
    );
  }
  for (let i = 0; i < flattenedRegions.length; i++) {
    if (JSON.stringify(plan.canonicalRegions[i]) !== JSON.stringify(flattenedRegions[i])) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        `Top-level canonicalRegions item at index ${i} ('${plan.canonicalRegions[i].regionId}') does not match page plan canonicalRegion ('${flattenedRegions[i].regionId}')`
      );
    }
  }

  const seenCanonicalRegionIds = new Set();
  for (const region of flattenedRegions) {
    if (seenCanonicalRegionIds.has(region.regionId)) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_DUPLICATE_REGION_ID,
        `Duplicate global canonical region ID found: '${region.regionId}'`
      );
    }
    seenCanonicalRegionIds.add(region.regionId);
  }

  const reqs = plan.acknowledgementRequirements;
  const acks = plan.acknowledgements;
  const expectedAcksSatisfied =
    (!reqs.rasterizedPagesRequired || acks.rasterizedPagesAccepted) &&
    (!reqs.textSearchLossRequired || acks.textSearchLossAccepted) &&
    (!reqs.accessibilityLossRequired || acks.accessibilityLossAccepted) &&
    (!reqs.annotationLossRequired || acks.annotationLossAccepted) &&
    (!reqs.formLossRequired || acks.formLossAccepted) &&
    (!reqs.signatureInvalidationRequired || acks.signatureInvalidationAccepted) &&
    (!reqs.encryptionChangeRequired || acks.encryptionChangeAccepted);

  if (plan.acknowledgementsSatisfied !== expectedAcksSatisfied) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `plan.acknowledgementsSatisfied (${plan.acknowledgementsSatisfied}) does not match expected (${expectedAcksSatisfied})`
    );
  }

  const expectedExecutable = (plan.blockers.length === 0 && plan.pagePlans.every(p => p.executable));
  if (plan.executable !== expectedExecutable) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `plan.executable (${plan.executable}) does not match expected (${expectedExecutable})`
    );
  }

  const expectedReady = expectedExecutable && expectedAcksSatisfied;
  if (plan.readyForExecution !== expectedReady) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      `plan.readyForExecution (${plan.readyForExecution}) does not match expected (${expectedReady})`
    );
  }

  const hasTopBlockers = plan.blockers.length > 0;
  const isEncrypted = plan.facts.encryption === 'present' || plan.facts.encryption === 'unknown';
  const isSigned = plan.facts.signatureDictionary === 'present' || plan.facts.signatureDictionary === 'unknown' ||
                   plan.facts.signatureField === 'present' || plan.facts.signatureField === 'unknown' ||
                   plan.facts.byteRange === 'present' || plan.facts.byteRange === 'unknown';
  const isXfa = plan.facts.xfa === 'present' || plan.facts.xfa === 'unknown';
  const isAcroform = plan.facts.acroForm === 'present' || plan.facts.acroForm === 'unknown';

  const hasAbsoluteDocumentBlockers =
    (isEncrypted && plan.policy.encryptionPolicy === 'block') ||
    (isSigned && plan.policy.signaturePolicy === 'block') ||
    isXfa ||
    (isAcroform && plan.policy.interactiveContentPolicy === 'block');

  if (plan.executable === true) {
    if (hasTopBlockers || hasAbsoluteDocumentBlockers) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_BLOCKED_DOCUMENT_FEATURE,
        'Plan executable cannot be true when document has blockers or blocked features'
      );
    }
    for (const pagePlan of plan.pagePlans) {
      if (pagePlan.executable === false) {
        throw new RedactionError(
          REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
          'Top-level plan executable cannot be true when any page plan executable is false'
        );
      }
      if (pagePlan.blockers && pagePlan.blockers.length > 0) {
        throw new RedactionError(
          REDACTION_ERROR_CODES.REDACTION_BLOCKED_DOCUMENT_FEATURE,
          'Top-level plan executable cannot be true when any page plan has blockers'
        );
      }
    }
  }
}

export function normalizeSecureRedactionPlanStructure(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'SecureRedactionPlan must be an object');
  }
  const jsonStr = JSON.stringify(raw);
  if (getUtf8ByteLength(jsonStr) > REDACTION_LIMITS.MAX_REPORT_SIZE_BYTES) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED,
      `Wire plan size (${getUtf8ByteLength(jsonStr)} bytes) exceeds MAX_REPORT_SIZE_BYTES limit (${REDACTION_LIMITS.MAX_REPORT_SIZE_BYTES} bytes)`
    );
  }

  assertNoUnknownFields(
    raw,
    [
      'schemaVersion', 'schema_version',
      'planVersion', 'plan_version',
      'inputDigest', 'input_digest', 'documentDigest', 'document_digest',
      'contentGeneration', 'content_generation',
      'planDigest', 'plan_digest',
      'policy',
      'acknowledgements',
      'acknowledgementRequirements', 'acknowledgement_requirements',
      'pagePlans', 'page_plans',
      'canonicalRegions', 'canonical_regions',
      'facts', 'documentFacts', 'document_facts',
      'losses', 'impact',
      'blockers',
      'warnings',
      'executable',
      'acknowledgementsSatisfied', 'acknowledgements_satisfied',
      'readyForExecution', 'ready_for_execution',
    ],
    'SecureRedactionPlan'
  );

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'SecureRedactionPlan');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED, `Unsupported plan schema version: '${schemaVersion}'`);
  }

  const planVersionRaw = readExclusiveAlias(raw, 'planVersion', 'plan_version', true, 'planVersion', 'SecureRedactionPlan');
  const planVersion = assertStrictString(planVersionRaw, 'planVersion');
  if (planVersion !== SECURE_REDACTION_PLAN_VERSION) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED, `Unsupported planVersion: '${planVersion}'`);
  }

  const inputDigestRaw = readMultiAlias(raw, ['inputDigest', 'input_digest', 'documentDigest', 'document_digest'], true, 'inputDigest', 'SecureRedactionPlan');
  const inputDigest = assertStrictString(inputDigestRaw, 'inputDigest');
  validateSha256Digest(inputDigest, 'inputDigest');

  const contentGenRaw = readExclusiveAlias(raw, 'contentGeneration', 'content_generation', true, 'contentGeneration', 'SecureRedactionPlan');
  const contentGeneration = assertStrictNumber(contentGenRaw, 'contentGeneration', { integer: true, min: 0 });

  const planDigestRaw = readExclusiveAlias(raw, 'planDigest', 'plan_digest', true, 'planDigest', 'SecureRedactionPlan');
  const planDigest = assertStrictString(planDigestRaw, 'planDigest');
  validateSha256Digest(planDigest, 'planDigest');

  const policy = normalizeSecureRedactionPolicy(raw.policy);
  const acknowledgements = normalizeRedactionAcknowledgements(raw.acknowledgements);

  const ackReqRaw = readExclusiveAlias(raw, 'acknowledgementRequirements', 'acknowledgement_requirements', true, 'acknowledgementRequirements', 'SecureRedactionPlan');
  const acknowledgementRequirements = normalizeRedactionAcknowledgementRequirements(ackReqRaw);

  const pagePlansRaw = readExclusiveAlias(raw, 'pagePlans', 'page_plans', true, 'pagePlans', 'SecureRedactionPlan');
  if (!Array.isArray(pagePlansRaw)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'pagePlans must be an array');
  }
  if (pagePlansRaw.length > REDACTION_LIMITS.MAX_PAGE_PLANS) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED, 'pagePlans count exceeds maximum limit');
  }
  const pagePlans = pagePlansRaw.map(normalizePageRedactionPlan);

  const canonRegRaw = readExclusiveAlias(raw, 'canonicalRegions', 'canonical_regions', true, 'canonicalRegions', 'SecureRedactionPlan');
  if (!Array.isArray(canonRegRaw)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'canonicalRegions must be an array');
  }
  const canonicalRegions = canonRegRaw.map(r => normalizeCanonicalRedactionRegion(r, undefined, policy.safetyMarginPoints));

  const factsRaw = readMultiAlias(raw, ['facts', 'documentFacts', 'document_facts'], true, 'facts', 'SecureRedactionPlan');
  const facts = normalizeDocumentRedactionFacts(factsRaw);

  const lossesRaw = readMultiAlias(raw, ['losses', 'impact'], true, 'losses', 'SecureRedactionPlan');
  const losses = normalizeRedactionFeatureLosses(lossesRaw);

  const blockers = validateUniqueBlockers(raw.blockers);
  const warnings = validateUniqueWarnings(raw.warnings);

  const execRaw = readExclusiveAlias(raw, 'executable', 'executable', true, 'executable', 'SecureRedactionPlan');
  const executable = assertStrictBoolean(execRaw, 'executable');

  const ackSatRaw = readExclusiveAlias(raw, 'acknowledgementsSatisfied', 'acknowledgements_satisfied', true, 'acknowledgementsSatisfied', 'SecureRedactionPlan');
  const acknowledgementsSatisfied = assertStrictBoolean(ackSatRaw, 'acknowledgementsSatisfied');

  const readyRaw = readExclusiveAlias(raw, 'readyForExecution', 'ready_for_execution', true, 'readyForExecution', 'SecureRedactionPlan');
  const readyForExecution = assertStrictBoolean(readyRaw, 'readyForExecution');

  const plan = {
    schemaVersion,
    planVersion,
    inputDigest,
    contentGeneration,
    planDigest,
    policy,
    acknowledgements,
    acknowledgementRequirements,
    pagePlans,
    canonicalRegions,
    facts,
    losses,
    blockers,
    warnings,
    executable,
    acknowledgementsSatisfied,
    readyForExecution,
  };

  assertNormalizedSecureRedactionPlan(plan);

  return plan;
}

export function normalizeSecureRedactionPlan(raw) {
  return normalizeSecureRedactionPlanStructure(raw);
}

export async function validateSecureRedactionPlan(rawOrPlan) {
  const verified = await normalizeAndVerifySecureRedactionPlan(rawOrPlan);
  return verified.digestVerified === true;
}

export function validateSecureRedactionPlanStructure(rawOrPlan) {
  normalizeSecureRedactionPlanStructure(rawOrPlan);
  return true;
}

// ---------------------------------------------------------------------------
// 9. Reports & Cross-Report Integrity Validations
// ---------------------------------------------------------------------------

/**
 * @typedef {Object} SecureRedactionTransformReport
 * @property {string} schemaVersion
 * @property {string} jobId
 * @property {string} sessionId
 * @property {string} inputDigest
 * @property {string} planDigest
 * @property {string} outputRelativeName
 * @property {string} outputDigest
 * @property {number} outputSizeBytes
 * @property {number} pageCount
 * @property {RedactionWarning[]} warnings
 * @property {'transformed_unverified'} status
 */

/**
 * @typedef {Object} SecureRedactionVerificationReport
 * @property {string} schemaVersion
 * @property {string} jobId
 * @property {string} sessionId
 * @property {string} planDigest
 * @property {string} transformDigest
 * @property {boolean} isVerified
 * @property {RedactionVerificationCheck[]} verificationChecks
 * @property {RedactionBlocker[]} blockers
 * @property {RedactionWarning[]} warnings
 */

/**
 * @typedef {Object} SecureRedactionExecutionReport
 * @property {string} schemaVersion
 * @property {string} jobId
 * @property {string} sessionId
 * @property {SecureRedactionTransformReport} transformReport
 * @property {SecureRedactionVerificationReport} verificationReport
 * @property {boolean} published
 * @property {SecureRedactionPublicationReport|null} publicationReport
 */

export function normalizeSecureRedactionTransformReport(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'Transform report must be an object');
  }
  let serializedReport;
  try {
    serializedReport = JSON.stringify(raw);
  } catch {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'Transform report must be JSON-serializable');
  }
  if (getUtf8ByteLength(serializedReport) > REDACTION_LIMITS.MAX_REPORT_SIZE_BYTES) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED,
      `Transform report size (${getUtf8ByteLength(serializedReport)} bytes) exceeds MAX_REPORT_SIZE_BYTES limit (${REDACTION_LIMITS.MAX_REPORT_SIZE_BYTES} bytes)`
    );
  }
  assertNoUnknownFields(
    raw,
    [
      'schemaVersion', 'schema_version',
      'jobId', 'job_id',
      'sessionId', 'session_id',
      'inputDigest', 'input_digest',
      'planDigest', 'plan_digest',
      'outputRelativeName', 'output_relative_name',
      'outputDigest', 'output_digest',
      'outputSizeBytes', 'output_size_bytes',
      'pageCount', 'page_count',
      'affectedPages', 'affected_pages',
      'unaffectedPages', 'unaffected_pages',
      'rasterDpi', 'raster_dpi',
      'appliedCanonicalRegions', 'applied_canonical_regions',
      'affectedPageRasterGeometry', 'affected_page_raster_geometry',
      'pageFeatureLosses', 'page_feature_losses',
      'backgroundPolicy', 'background_policy',
      'warnings',
      'transformEvidence', 'transform_evidence',
      'status',
    ],
    'SecureRedactionTransformReport'
  );

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'SecureRedactionTransformReport');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED, `Unsupported transform report schema version: '${schemaVersion}'`);
  }

  const jobIdRaw = readExclusiveAlias(raw, 'jobId', 'job_id', true, 'jobId', 'SecureRedactionTransformReport');
  const jobId = assertStrictString(jobIdRaw, 'jobId', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_IDENTITY_LEN });

  const sessionIdRaw = readExclusiveAlias(raw, 'sessionId', 'session_id', true, 'sessionId', 'SecureRedactionTransformReport');
  const sessionId = assertStrictString(sessionIdRaw, 'sessionId', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_IDENTITY_LEN });

  const inputDigestRaw = readExclusiveAlias(raw, 'inputDigest', 'input_digest', true, 'inputDigest', 'SecureRedactionTransformReport');
  const inputDigest = assertStrictString(inputDigestRaw, 'inputDigest');
  validateSha256Digest(inputDigest, 'inputDigest');

  const planDigestRaw = readExclusiveAlias(raw, 'planDigest', 'plan_digest', true, 'planDigest', 'SecureRedactionTransformReport');
  const planDigest = assertStrictString(planDigestRaw, 'planDigest');
  validateSha256Digest(planDigest, 'planDigest');

  const outputRelativeNameRaw = readExclusiveAlias(raw, 'outputRelativeName', 'output_relative_name', true, 'outputRelativeName', 'SecureRedactionTransformReport');
  const outputRelativeName = assertStrictString(outputRelativeNameRaw, 'outputRelativeName', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_PATH_LEN });
  if (outputRelativeName !== 'secure-redaction-transform.pdf') throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'outputRelativeName must be the fixed internal artifact name');

  const outputDigestRaw = readExclusiveAlias(raw, 'outputDigest', 'output_digest', true, 'outputDigest', 'SecureRedactionTransformReport');
  const outputDigest = assertStrictString(outputDigestRaw, 'outputDigest');
  validateSha256Digest(outputDigest, 'outputDigest');

  const outputSizeBytes = assertStrictNumber(
    readExclusiveAlias(raw, 'outputSizeBytes', 'output_size_bytes', true, 'outputSizeBytes', 'SecureRedactionTransformReport'),
    'outputSizeBytes',
    { integer: true, min: 1, max: REDACTION_LIMITS.MAX_SECURE_REDACTION_OUTPUT_BYTES },
    REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED
  );
  const pageCount = assertU32(readExclusiveAlias(raw, 'pageCount', 'page_count', true, 'pageCount', 'SecureRedactionTransformReport'), 'pageCount');
  const affectedPages = readExclusiveAlias(raw, 'affectedPages', 'affected_pages', true, 'affectedPages', 'SecureRedactionTransformReport');
  const unaffectedPages = readExclusiveAlias(raw, 'unaffectedPages', 'unaffected_pages', true, 'unaffectedPages', 'SecureRedactionTransformReport');
  const rasterDpi = assertU32(readExclusiveAlias(raw, 'rasterDpi', 'raster_dpi', true, 'rasterDpi', 'SecureRedactionTransformReport'), 'rasterDpi');
  const affectedPageSet = new Set();
  const unaffectedPageSet = new Set();
  for (const [name, pages, seen] of [['affectedPages', affectedPages, affectedPageSet], ['unaffectedPages', unaffectedPages, unaffectedPageSet]]) {
    if (!Array.isArray(pages) || pages.length > REDACTION_LIMITS.MAX_PAGE_PLANS) throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED, `${name} invalid`);
    pages.forEach((page, index) => {
      assertU32(page, `${name}[${index}]`);
      if (page >= pageCount || seen.has(page)) throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, `${name} must contain unique in-range page indexes`);
      seen.add(page);
    });
  }
  if (affectedPages.length + unaffectedPages.length !== pageCount || [...affectedPageSet].some(page => unaffectedPageSet.has(page))) throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'Transform page collections must be a complete disjoint partition');
  const appliedCanonicalRegions = readExclusiveAlias(raw, 'appliedCanonicalRegions', 'applied_canonical_regions', true, 'appliedCanonicalRegions', 'SecureRedactionTransformReport');
  const affectedPageRasterGeometry = readExclusiveAlias(raw, 'affectedPageRasterGeometry', 'affected_page_raster_geometry', true, 'affectedPageRasterGeometry', 'SecureRedactionTransformReport');
  const pageFeatureLosses = readExclusiveAlias(raw, 'pageFeatureLosses', 'page_feature_losses', true, 'pageFeatureLosses', 'SecureRedactionTransformReport');
  if (!Array.isArray(appliedCanonicalRegions) || appliedCanonicalRegions.length > REDACTION_LIMITS.MAX_REGIONS_PER_INPUT || !Array.isArray(affectedPageRasterGeometry) || !Array.isArray(pageFeatureLosses) || affectedPageRasterGeometry.length !== affectedPages.length || pageFeatureLosses.length !== pageCount) throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'Transform evidence collections are inconsistent');
  const normalizedRegions = appliedCanonicalRegions.map(region => normalizeCanonicalRedactionRegion(region, undefined, undefined));
  const regionIds = new Set();
  const sourceRegionIds = new Set();
  for (const region of normalizedRegions) {
    if (!affectedPageSet.has(region.pageIndex)) throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'Applied canonical regions must belong to affected pages');
    if (regionIds.has(region.regionId)) throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_DUPLICATE_REGION_ID, `Duplicate applied canonical region ID '${region.regionId}'`);
    regionIds.add(region.regionId);
    for (const sourceRegionId of region.sourceRegionIds) {
      if (sourceRegionIds.has(sourceRegionId)) throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_DUPLICATE_REGION_ID, `Duplicate applied source region ID '${sourceRegionId}'`);
      sourceRegionIds.add(sourceRegionId);
    }
  }
  const rasterEvidencePages = new Set();
  const normalizedRasterGeometry = affectedPageRasterGeometry.map((entry, index) => {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, `affectedPageRasterGeometry[${index}] must be an object`);
    assertNoUnknownFields(entry, ['pageIndex', 'page_index', 'width', 'height', 'dpi'], 'TransformRasterPageEvidence');
    const pageIndex = assertU32(readExclusiveAlias(entry, 'pageIndex', 'page_index', true, 'pageIndex', 'TransformRasterPageEvidence'), 'pageIndex');
    const width = assertPositiveU32(entry.width, 'width', REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED);
    const height = assertPositiveU32(entry.height, 'height', REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED);
    const dpi = assertStrictNumber(entry.dpi, 'dpi', { integer: true, min: REDACTION_LIMITS.MIN_RASTER_DPI, max: REDACTION_LIMITS.MAX_RASTER_DPI });
    if (!affectedPageSet.has(pageIndex) || rasterEvidencePages.has(pageIndex) || dpi !== rasterDpi) throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'Raster evidence must uniquely cover affected pages at the report DPI');
    if (width > REDACTION_LIMITS.MAX_RASTER_DIMENSION || height > REDACTION_LIMITS.MAX_RASTER_DIMENSION) throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED, 'Raster evidence exceeds maximum raster dimension');
    if (width * height > REDACTION_LIMITS.MAX_PIXELS_PER_PAGE) throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_RESOURCE_LIMIT_EXCEEDED, 'Raster evidence exceeds maximum page pixel count');
    rasterEvidencePages.add(pageIndex);
    return { pageIndex, width, height, dpi };
  });
  const featureLossPages = new Set();
  const normalizedFeatureLosses = pageFeatureLosses.map((entry, index) => {
    if (!entry || typeof entry !== 'object' || Array.isArray(entry)) throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, `pageFeatureLosses[${index}] must be an object`);
    assertNoUnknownFields(entry, ['pageIndex', 'page_index', 'featureLosses', 'feature_losses'], 'TransformPageFeatureLoss');
    const pageIndex = assertU32(readExclusiveAlias(entry, 'pageIndex', 'page_index', true, 'pageIndex', 'TransformPageFeatureLoss'), 'pageIndex');
    if (pageIndex >= pageCount || featureLossPages.has(pageIndex)) throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'Feature-loss evidence must uniquely cover every page');
    featureLossPages.add(pageIndex);
    return { pageIndex, featureLosses: normalizeRedactionFeatureLosses(readExclusiveAlias(entry, 'featureLosses', 'feature_losses', true, 'featureLosses', 'TransformPageFeatureLoss')) };
  });
  const backgroundPolicy = assertStrictString(readExclusiveAlias(raw, 'backgroundPolicy', 'background_policy', true, 'backgroundPolicy', 'SecureRedactionTransformReport'), 'backgroundPolicy');
  if (backgroundPolicy !== 'opaque_white_v1') throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'Unsupported transform background policy');
  const transformEvidence = readExclusiveAlias(raw, 'transformEvidence', 'transform_evidence', true, 'transformEvidence', 'SecureRedactionTransformReport');
  if (!transformEvidence || typeof transformEvidence !== 'object' || Array.isArray(transformEvidence)) throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'transformEvidence must be an object');
  const evidenceFields = [
    ['newDocumentConstructed', 'new_document_constructed'], ['affectedPagesRasterized', 'affected_pages_rasterized'],
    ['affectedPageOriginalObjectsNotImported', 'affected_page_original_objects_not_imported'], ['unaffectedPagesImported', 'unaffected_pages_imported'],
    ['fullRewriteUsed', 'full_rewrite_used'], ['incrementalSaveNotUsed', 'incremental_save_not_used'],
    ['opaqueWhiteBackgroundPolicy', 'opaque_white_background_policy'], ['losslessRasterPath', 'lossless_raster_path'], ['outputDigestCalculated', 'output_digest_calculated'],
  ];
  assertNoUnknownFields(transformEvidence, evidenceFields.flat(), 'SecureRedactionTransformEvidence');
  const normalizedEvidence = {};
  for (const [camel, snake] of evidenceFields) normalizedEvidence[camel] = assertStrictBoolean(readExclusiveAlias(transformEvidence, camel, snake, true, camel, 'SecureRedactionTransformEvidence'), camel);
  if (!normalizedEvidence.newDocumentConstructed || !normalizedEvidence.fullRewriteUsed || !normalizedEvidence.incrementalSaveNotUsed || !normalizedEvidence.opaqueWhiteBackgroundPolicy || !normalizedEvidence.losslessRasterPath || !normalizedEvidence.outputDigestCalculated || (affectedPageSet.size > 0 && (!normalizedEvidence.affectedPagesRasterized || !normalizedEvidence.affectedPageOriginalObjectsNotImported)) || (unaffectedPageSet.size > 0 && !normalizedEvidence.unaffectedPagesImported)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'Transform evidence contradicts the report collections or required transform path');
  }

  const warnings = validateUniqueWarnings(raw.warnings);
  const status = assertStrictString(raw.status, 'status');

  if (status !== 'transformed_unverified') {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'Transform report status must be "transformed_unverified"'
    );
  }

  return {
    schemaVersion,
    jobId,
    sessionId,
    inputDigest,
    planDigest,
    outputRelativeName,
    outputDigest,
    outputSizeBytes,
    pageCount,
    affectedPages,
    unaffectedPages,
    rasterDpi,
    appliedCanonicalRegions: normalizedRegions,
    affectedPageRasterGeometry: normalizedRasterGeometry,
    pageFeatureLosses: normalizedFeatureLosses,
    backgroundPolicy,
    warnings,
    transformEvidence: normalizedEvidence,
    status,
  };
}

export function validateSecureRedactionTransformReport(rawOrReport) {
  normalizeSecureRedactionTransformReport(rawOrReport);
  return true;
}

export function normalizeSecureRedactionVerificationReport(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'Verification report must be an object');
  }
  assertNoUnknownFields(
    raw,
    [
      'schemaVersion', 'schema_version',
      'verificationVersion', 'verification_version',
      'jobId', 'job_id',
      'sessionId', 'session_id',
      'planDigest', 'plan_digest',
      'transformDigest', 'transform_digest',
      'isVerified', 'is_verified',
      'verificationChecks', 'verification_checks',
      'blockers',
      'warnings',
    ],
    'SecureRedactionVerificationReport'
  );

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'SecureRedactionVerificationReport');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED, `Unsupported verification report schema version: '${schemaVersion}'`);
  }

  const verificationVersionRaw = readExclusiveAlias(raw, 'verificationVersion', 'verification_version', true, 'verificationVersion', 'SecureRedactionVerificationReport');
  const verificationVersion = assertStrictString(verificationVersionRaw, 'verificationVersion');
  if (verificationVersion !== SECURE_REDACTION_VERIFICATION_VERSION) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED, `Unsupported secure redaction verification version: '${verificationVersion}'`);
  }

  const jobIdRaw = readExclusiveAlias(raw, 'jobId', 'job_id', true, 'jobId', 'SecureRedactionVerificationReport');
  const jobId = assertStrictString(jobIdRaw, 'jobId', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_IDENTITY_LEN });

  const sessionIdRaw = readExclusiveAlias(raw, 'sessionId', 'session_id', true, 'sessionId', 'SecureRedactionVerificationReport');
  const sessionId = assertStrictString(sessionIdRaw, 'sessionId', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_IDENTITY_LEN });

  const planDigestRaw = readExclusiveAlias(raw, 'planDigest', 'plan_digest', true, 'planDigest', 'SecureRedactionVerificationReport');
  const planDigest = assertStrictString(planDigestRaw, 'planDigest');
  validateSha256Digest(planDigest, 'planDigest');

  const transformDigestRaw = readExclusiveAlias(raw, 'transformDigest', 'transform_digest', true, 'transformDigest', 'SecureRedactionVerificationReport');
  const transformDigest = assertStrictString(transformDigestRaw, 'transformDigest');
  validateSha256Digest(transformDigest, 'transformDigest');

  const isVerifiedRaw = readExclusiveAlias(raw, 'isVerified', 'is_verified', true, 'isVerified', 'SecureRedactionVerificationReport');
  const isVerified = assertStrictBoolean(isVerifiedRaw, 'isVerified');

  const rawChecks = readExclusiveAlias(raw, 'verificationChecks', 'verification_checks', true, 'verificationChecks', 'SecureRedactionVerificationReport');
  const verificationChecks = validateUniqueVerificationChecks(rawChecks);
  if (verificationChecks.length !== REQUIRED_SECURE_REDACTION_VERIFICATION_CHECKS.length ||
      verificationChecks.some((item, index) => item.code !== REQUIRED_SECURE_REDACTION_VERIFICATION_CHECKS[index])) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'Verification report must contain the exact canonical required check set in order');
  }

  const blockers = validateUniqueBlockers(raw.blockers);
  const warnings = validateUniqueWarnings(raw.warnings);

  // `isVerified` is a biconditional protocol value, not an advisory flag.
  const expectedVerified = blockers.length === 0 && verificationChecks.every((check) => check.status === 'passed');
  if (isVerified !== expectedVerified) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'Verification report isVerified must equal (all required checks passed AND blockers empty)'
    );
  }

  return {
    schemaVersion,
    verificationVersion,
    jobId,
    sessionId,
    planDigest,
    transformDigest,
    isVerified,
    verificationChecks,
    blockers,
    warnings,
  };
}

export function validateSecureRedactionVerificationReport(rawOrReport) {
  normalizeSecureRedactionVerificationReport(rawOrReport);
  return true;
}

export function normalizeSecureRedactionExecutionReport(raw) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY, 'Execution report must be an object');
  }
  assertNoUnknownFields(
    raw,
    [
      'schemaVersion', 'schema_version',
      'jobId', 'job_id',
      'sessionId', 'session_id',
      'transformReport', 'transform_report',
      'verificationReport', 'verification_report',
      'published',
      'publicationReport', 'publication_report',
    ],
    'SecureRedactionExecutionReport'
  );

  const schemaVersionRaw = readExclusiveAlias(raw, 'schemaVersion', 'schema_version', true, 'schemaVersion', 'SecureRedactionExecutionReport');
  const schemaVersion = assertStrictString(schemaVersionRaw, 'schemaVersion');
  if (schemaVersion !== REDACTION_SCHEMA_VERSION) {
    throw new RedactionError(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED, `Unsupported execution report schema version: '${schemaVersion}'`);
  }

  const jobIdRaw = readExclusiveAlias(raw, 'jobId', 'job_id', true, 'jobId', 'SecureRedactionExecutionReport');
  const jobId = assertStrictString(jobIdRaw, 'jobId', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_IDENTITY_LEN });

  const sessionIdRaw = readExclusiveAlias(raw, 'sessionId', 'session_id', true, 'sessionId', 'SecureRedactionExecutionReport');
  const sessionId = assertStrictString(sessionIdRaw, 'sessionId', { minLen: 1, maxLen: REDACTION_LIMITS.MAX_IDENTITY_LEN });

  const transformReportRaw = readExclusiveAlias(raw, 'transformReport', 'transform_report', true, 'transformReport', 'SecureRedactionExecutionReport');
  const transformReport = normalizeSecureRedactionTransformReport(transformReportRaw);

  const verificationReportRaw = readExclusiveAlias(raw, 'verificationReport', 'verification_report', true, 'verificationReport', 'SecureRedactionExecutionReport');
  const verificationReport = normalizeSecureRedactionVerificationReport(verificationReportRaw);

  const published = assertStrictBoolean(raw.published, 'published');

  const pubReportRaw = readExclusiveAlias(raw, 'publicationReport', 'publication_report', false, 'publicationReport', 'SecureRedactionExecutionReport');
  const publicationReport = normalizeNullableObject(pubReportRaw, normalizeSecureRedactionPublicationReport, 'publicationReport');

  // Cross-Report Integrity & Identity Invariants
  if (jobId !== transformReport.jobId || jobId !== verificationReport.jobId) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'Cross-report mismatch: jobId does not match across execution, transform, and verification reports'
    );
  }
  if (sessionId !== transformReport.sessionId || sessionId !== verificationReport.sessionId) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
      'Cross-report mismatch: sessionId does not match across execution, transform, and verification reports'
    );
  }
  if (verificationReport.planDigest !== transformReport.planDigest) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_PLAN_DIGEST_MISMATCH,
      'Cross-report mismatch: verificationReport.planDigest does not match transformReport.planDigest'
    );
  }
  if (verificationReport.transformDigest !== transformReport.outputDigest) {
    throw new RedactionError(
      REDACTION_ERROR_CODES.REDACTION_PLAN_DIGEST_MISMATCH,
      'Cross-report mismatch: verificationReport.transformDigest does not match transformReport.outputDigest'
    );
  }

  if (published) {
    if (!publicationReport) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        'Execution report published is true but publicationReport is missing/null'
      );
    }
    if (publicationReport.publishedDigest !== transformReport.outputDigest) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_PLAN_DIGEST_MISMATCH,
        'Publication mismatch: publicationReport.publishedDigest does not match transformReport.outputDigest'
      );
    }
    if (!verificationReport.isVerified) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        'Execution report cannot be published if verificationReport.isVerified is false'
      );
    }
    if (verificationReport.blockers.length > 0) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        'Execution report cannot be published if verificationReport has blockers'
      );
    }
  } else {
    if (publicationReport !== null) {
      throw new RedactionError(
        REDACTION_ERROR_CODES.REDACTION_INVALID_POLICY,
        'Execution report published is false so publicationReport must be null'
      );
    }
  }

  return {
    schemaVersion,
    jobId,
    sessionId,
    transformReport,
    verificationReport,
    published,
    publicationReport,
  };
}

export function validateSecureRedactionExecutionReport(rawOrReport) {
  normalizeSecureRedactionExecutionReport(rawOrReport);
  return true;
}
