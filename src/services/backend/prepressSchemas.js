const REPORT_SCHEMA = "print-resource-inspection/1";
const PREVIEW_SCHEMA = "rgb-derived-print-preview/1";
const PREVIEW_METHODOLOGY = "naive-rgb-to-cmyk/1";
const SUMMARY_STATUSES = new Set([
  "findings_detected",
  "no_findings_in_implemented_checks",
  "inspection_incomplete",
  "inspection_failed",
]);
const COLOR_SPACE_FAMILIES = new Set([
  "device_gray", "device_rgb", "device_cmyk", "cal_gray", "cal_rgb", "lab",
  "icc_based", "indexed", "pattern", "separation", "device_n", "n_channel",
  "unknown_custom",
]);
const COVERAGE_STATUSES = new Set(["inspected", "not_inspected", "incomplete"]);
const MAX_DIMENSION = 16_384;
const MAX_PIXELS = 24_000_000;
const MAX_PNG_BYTES = 24 * 1024 * 1024;
const MAX_TOTAL_BYTES = 72 * 1024 * 1024;
const MAX_REPORT_PAGES = 10_000;
const MAX_COLOR_SPACES = 20_000;
const MAX_ICC_PROFILES = 256;
const MAX_OUTPUT_INTENTS = 64;
const MAX_SPOT_COLORS = 4_096;
const MAX_FONTS = 20_000;
const MAX_FINDINGS = 4_096;

const fail = (message) => {
  const error = new Error(`Geçersiz print inspection/preview yanıtı: ${message}`);
  error.code = "PREPRESS_RESPONSE_SCHEMA_INVALID";
  throw error;
};
const object = (value, name) => {
  if (!value || typeof value !== "object" || Array.isArray(value)) fail(`${name} object olmalıdır`);
  return value;
};
const array = (value, name) => {
  if (!Array.isArray(value)) fail(`${name} array olmalıdır`);
  return value;
};
const string = (value, name) => {
  if (typeof value !== "string") fail(`${name} string olmalıdır`);
  return value;
};
const finite = (value, name) => {
  if (typeof value !== "number" || !Number.isFinite(value)) fail(`${name} finite sayı olmalıdır`);
  return value;
};
const integer = (value, name, min = 0, max = Number.MAX_SAFE_INTEGER) => {
  if (!Number.isSafeInteger(value) || value < min || value > max) fail(`${name} güvenli integer sınırında olmalıdır`);
  return value;
};
const bool = (value, name) => {
  if (typeof value !== "boolean") fail(`${name} boolean olmalıdır`);
  return value;
};
const stringArray = (value, name) => array(value, name).map((entry, index) => string(entry, `${name}[${index}]`));
const optionalString = (value, name) => value === null ? null : string(value, name);
const optionalInteger = (value, name, min = 0, max = Number.MAX_SAFE_INTEGER) => value === null ? null : integer(value, name, min, max);
const optionalBool = (value, name) => value === null ? null : bool(value, name);
const boundedArray = (value, name, max) => {
  const result = array(value, name);
  if (result.length > max) fail(`${name} eleman limiti aşıldı`);
  return result;
};
const affectedPages = (value, name) => boundedArray(value, name, MAX_REPORT_PAGES).map((page, index) => integer(page, `${name}[${index}]`, 0, MAX_REPORT_PAGES - 1));

function validateIdentity(identity, expected) {
  object(identity, "identity");
  string(identity.sessionId, "identity.sessionId");
  integer(identity.contentGeneration, "identity.contentGeneration");
  integer(identity.physicalPageIndex, "identity.physicalPageIndex");
  string(identity.requestId, "identity.requestId");
  string(identity.methodologyVersion, "identity.methodologyVersion");
  if (expected && (
    identity.sessionId !== expected.sessionId ||
    identity.contentGeneration !== expected.contentGeneration ||
    identity.physicalPageIndex !== expected.physicalPageIndex ||
    identity.requestId !== expected.requestId
  )) fail("identity aktif session/generation/page/request ile eşleşmiyor");
}

function decodedBase64Bytes(value, name) {
  string(value, name);
  if (value.length === 0 || value.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(value)) {
    fail(`${name} canonical base64 biçiminde olmalıdır`);
  }
  const padding = value.endsWith("==") ? 2 : value.endsWith("=") ? 1 : 0;
  return (value.length / 4) * 3 - padding;
}

function validateMetadata(metadata) {
  object(metadata, "metadata");
  if (metadata.schemaVersion !== PREVIEW_SCHEMA) fail(`bilinmeyen preview schema: ${metadata.schemaVersion}`);
  if (metadata.methodology !== PREVIEW_METHODOLOGY) fail(`bilinmeyen methodology: ${metadata.methodology}`);
  string(metadata.sourceColorModel, "metadata.sourceColorModel");
  if (metadata.sourceColorModel !== "pdfium_display_rgb") fail("sourceColorModel desteklenmiyor");
  if (bool(metadata.approximate, "metadata.approximate") !== true) fail("approximate true olmalıdır");
  for (const key of ["colorManaged", "outputIntentApplied", "spotColorsPreserved", "overprintSimulated"]) {
    if (bool(metadata[key], `metadata.${key}`) !== false) fail(`${key} false olmalıdır`);
  }
  if (metadata.renderingIntent !== "not_applied") fail("renderingIntent not_applied olmalıdır");
  stringArray(metadata.warnings, "metadata.warnings");
}

function validateDimensions(value) {
  const width = integer(value.width, "width", 1, MAX_DIMENSION);
  const height = integer(value.height, "height", 1, MAX_DIMENSION);
  if (width * height > MAX_PIXELS) fail("preview pixel limiti aşıldı");
  integer(value.pageIndex, "pageIndex");
  bool(value.truncated, "truncated");
}

export function validatePrintResourceInspectionReport(value, expected) {
  object(value, "report");
  if (value.schemaVersion !== REPORT_SCHEMA) fail(`bilinmeyen inspection schema: ${value.schemaVersion}`);
  if (string(value.inspectionVersion, "inspectionVersion") !== "print-resource-inspection/1.0") fail("inspectionVersion desteklenmiyor");
  validateIdentity(value.identity, expected);
  if (value.identity.methodologyVersion !== value.inspectionVersion) fail("inspection identity methodologyVersion eşleşmiyor");
  if (!SUMMARY_STATUSES.has(value.summaryStatus)) fail("summaryStatus bilinmiyor");
  if (bool(value.conformanceClaim, "conformanceClaim") !== false) fail("conformanceClaim false olmalıdır");
  object(value.documentFacts, "documentFacts");
  string(value.documentFacts.pdfVersion, "documentFacts.pdfVersion");
  integer(value.documentFacts.pageCount, "documentFacts.pageCount", 0, MAX_REPORT_PAGES);
  bool(value.documentFacts.encrypted, "documentFacts.encrypted");
  boundedArray(value.colorSpaces, "colorSpaces", MAX_COLOR_SPACES).forEach((entry, index) => {
    object(entry, `colorSpaces[${index}]`);
    string(entry.id, `colorSpaces[${index}].id`);
    optionalString(entry.objectId, `colorSpaces[${index}].objectId`);
    optionalString(entry.resourceName, `colorSpaces[${index}].resourceName`);
    if (!COLOR_SPACE_FAMILIES.has(entry.family)) fail(`colorSpaces[${index}].family bilinmiyor`);
    integer(entry.definitionCount, `colorSpaces[${index}].definitionCount`);
    integer(entry.referenceCount, `colorSpaces[${index}].referenceCount`);
    affectedPages(entry.affectedPages, `colorSpaces[${index}].affectedPages`);
    optionalString(entry.alternateFamily, `colorSpaces[${index}].alternateFamily`);
    if (entry.usageStatus !== "usage_not_checked") fail("color-space usageStatus desteklenmiyor");
  });
  boundedArray(value.iccProfiles, "iccProfiles", MAX_ICC_PROFILES).forEach((entry, index) => {
    object(entry, `iccProfiles[${index}]`);
    optionalString(entry.objectId, `iccProfiles[${index}].objectId`);
    if (entry.inspectionLevel !== "basic_header_only") fail("ICC inspectionLevel desteklenmiyor");
    bool(entry.streamPresent, `iccProfiles[${index}].streamPresent`);
    integer(entry.encodedLength, `iccProfiles[${index}].encodedLength`);
    optionalInteger(entry.decodedLength, `iccProfiles[${index}].decodedLength`);
    optionalString(entry.sha256, `iccProfiles[${index}].sha256`);
    optionalInteger(entry.componentCount, `iccProfiles[${index}].componentCount`, 0, 32);
    for (const key of ["alternateColorSpace", "profileVersion", "deviceClass", "inputColorSpace", "pcs"]) optionalString(entry[key], `iccProfiles[${index}].${key}`);
    optionalInteger(entry.declaredSize, `iccProfiles[${index}].declaredSize`, 0, 0xffff_ffff);
    optionalBool(entry.signatureAcsp, `iccProfiles[${index}].signatureAcsp`);
    string(entry.status, `iccProfiles[${index}].status`);
    stringArray(entry.warnings, `iccProfiles[${index}].warnings`);
  });
  boundedArray(value.outputIntents, "outputIntents", MAX_OUTPUT_INTENTS).forEach((entry, index) => {
    object(entry, `outputIntents[${index}]`);
    for (const key of ["objectId", "typeName", "subtype", "outputConditionIdentifier", "registryName", "profileObservationId"]) optionalString(entry[key], `outputIntents[${index}].${key}`);
    for (const key of ["dictionaryPresent", "hasDestOutputProfile", "hasExternalProfileReference"]) bool(entry[key], `outputIntents[${index}].${key}`);
    if (entry.status !== "observed_not_validated") fail("OutputIntent status desteklenmiyor");
    stringArray(entry.warnings, `outputIntents[${index}].warnings`);
  });
  boundedArray(value.spotColors, "spotColors", MAX_SPOT_COLORS).forEach((entry, index) => {
    object(entry, `spotColors[${index}]`);
    optionalString(entry.objectId, `spotColors[${index}].objectId`);
    if (!new Set(["separation", "device_n", "n_channel"]).has(entry.family)) fail("spot family desteklenmiyor");
    stringArray(entry.colorantNames, `spotColors[${index}].colorantNames`);
    for (const key of ["containsAll", "containsNone", "tintTransformPresent", "attributesPresent"]) bool(entry[key], `spotColors[${index}].${key}`);
    optionalString(entry.alternateFamily, `spotColors[${index}].alternateFamily`);
    affectedPages(entry.affectedPages, `spotColors[${index}].affectedPages`);
    if (entry.usageStatus !== "usage_not_checked") fail("spot usageStatus desteklenmiyor");
    stringArray(entry.warnings, `spotColors[${index}].warnings`);
  });
  boundedArray(value.transparency, "transparency", MAX_FINDINGS).forEach((entry, index) => {
    object(entry, `transparency[${index}]`);
    optionalString(entry.objectId, `transparency[${index}].objectId`);
    string(entry.kind, `transparency[${index}].kind`);
    stringArray(entry.keys, `transparency[${index}].keys`);
    affectedPages(entry.affectedPages, `transparency[${index}].affectedPages`);
    if (entry.usageStatus !== "usage_not_checked") fail("transparency usageStatus desteklenmiyor");
  });
  boundedArray(value.fonts, "fonts", MAX_FONTS).forEach((entry, index) => {
    object(entry, `fonts[${index}]`);
    optionalString(entry.objectId, `fonts[${index}].objectId`);
    string(entry.resourceName, `fonts[${index}].resourceName`);
    optionalString(entry.subtype, `fonts[${index}].subtype`);
    optionalString(entry.baseFont, `fonts[${index}].baseFont`);
    bool(entry.embeddedStreamPresent, `fonts[${index}].embeddedStreamPresent`);
    bool(entry.subsetNameIndication, `fonts[${index}].subsetNameIndication`);
    affectedPages(entry.affectedPages, `fonts[${index}].affectedPages`);
    if (entry.usageStatus !== "usage_not_checked") fail("font usageStatus desteklenmiyor");
    stringArray(entry.warnings, `fonts[${index}].warnings`);
  });
  boundedArray(value.pageBoxes, "pageBoxes", MAX_REPORT_PAGES).forEach((page, index) => {
    object(page, `pageBoxes[${index}]`);
    integer(page.physicalPageIndex, `pageBoxes[${index}].physicalPageIndex`, 0, MAX_REPORT_PAGES - 1);
    finite(page.rotationDegrees, `pageBoxes[${index}].rotationDegrees`);
    for (const key of ["mediaBox", "cropBox", "bleedBox", "trimBox", "artBox"]) {
      const box = object(page[key], `pageBoxes[${index}].${key}`);
      if (box.coordinates !== null) {
        if (!Array.isArray(box.coordinates) || box.coordinates.length !== 4) fail(`${key}.coordinates dört sayı olmalıdır`);
        box.coordinates.forEach((coordinate) => finite(coordinate, `${key}.coordinate`));
      }
      string(box.source, `${key}.source`);
      bool(box.validGeometry, `${key}.validGeometry`);
      bool(box.nonZeroOrigin, `${key}.nonZeroOrigin`);
      bool(box.negativeOrigin, `${key}.negativeOrigin`);
    }
  });
  stringArray(value.warnings, "warnings");
  const coverage = object(value.coverage, "coverage");
  for (const name of ["inspectedAreas", "notInspectedAreas", "incompleteAreas", "limitations"]) stringArray(coverage[name], `coverage.${name}`);
  integer(coverage.pagesInspected, "coverage.pagesInspected");
  object(coverage.resourceCounts, "coverage.resourceCounts");
  for (const key of ["visitedObjects", "resourceReferences", "xobjects", "patterns", "shadings", "colorSpaceDefinitions", "uniqueColorSpaceObjects", "iccProfiles", "outputIntents", "spotColors", "fonts", "transparencyObservations"]) {
    integer(coverage.resourceCounts[key], `coverage.resourceCounts.${key}`);
  }
  array(coverage.areas, "coverage.areas").forEach((entry) => {
    object(entry, "coverage.area");
    string(entry.area, "coverage.area.area");
    if (!COVERAGE_STATUSES.has(entry.status)) fail("coverage area status bilinmiyor");
  });
  bool(value.truncated, "truncated");
  return value;
}

export function validateRgbDerivedCoveragePreview(value, expected) {
  object(value, "coveragePreview");
  validateIdentity(value.identity, expected);
  if (value.identity.methodologyVersion !== PREVIEW_METHODOLOGY) fail("preview identity methodologyVersion eşleşmiyor");
  validateMetadata(value.metadata);
  validateDimensions(value);
  if (value.pageIndex !== value.identity.physicalPageIndex) fail("preview pageIndex identity ile eşleşmiyor");
  const threshold = finite(value.thresholdPercent, "thresholdPercent");
  if (threshold < 0 || threshold > 400) fail("thresholdPercent 0..400 aralığında olmalıdır");
  const bytes = integer(value.encodedPngBytes, "encodedPngBytes", 1, MAX_PNG_BYTES);
  if (decodedBase64Bytes(value.heatmapPngBase64, "heatmapPngBase64") !== bytes) fail("coverage PNG size metadata eşleşmiyor");
  const statistics = object(value.pixelStatistics, "pixelStatistics");
  integer(statistics.totalPixels, "pixelStatistics.totalPixels", 1, MAX_PIXELS);
  if (statistics.totalPixels !== value.width * value.height) fail("pixelStatistics.totalPixels boyutlarla eşleşmiyor");
  integer(statistics.pixelsAboveThreshold, "pixelStatistics.pixelsAboveThreshold", 0, statistics.totalPixels);
  finite(statistics.maximumEstimatedChannelSumPercent, "maximumEstimatedChannelSumPercent");
  finite(statistics.averageEstimatedChannelSumPercent, "averageEstimatedChannelSumPercent");
  return value;
}

export function validateRgbDerivedProcessPreview(value, expected) {
  object(value, "processPreview");
  validateIdentity(value.identity, expected);
  if (value.identity.methodologyVersion !== PREVIEW_METHODOLOGY) fail("preview identity methodologyVersion eşleşmiyor");
  validateMetadata(value.metadata);
  validateDimensions(value);
  if (value.pageIndex !== value.identity.physicalPageIndex) fail("preview pageIndex identity ile eşleşmiyor");
  if (!Array.isArray(value.encodedPngBytes) || value.encodedPngBytes.length !== 4) fail("encodedPngBytes dört elemanlı olmalıdır");
  const fields = ["cPreviewPngBase64", "mPreviewPngBase64", "yPreviewPngBase64", "kPreviewPngBase64"];
  let total = 0;
  fields.forEach((field, index) => {
    const size = integer(value.encodedPngBytes[index], `encodedPngBytes[${index}]`, 1, MAX_PNG_BYTES);
    if (decodedBase64Bytes(value[field], field) !== size) fail(`${field} size metadata eşleşmiyor`);
    total += size;
  });
  if (integer(value.totalEncodedPngBytes, "totalEncodedPngBytes", 1, MAX_TOTAL_BYTES) !== total) fail("totalEncodedPngBytes eşleşmiyor");
  return value;
}
