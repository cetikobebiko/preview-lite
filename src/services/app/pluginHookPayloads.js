// Hook payload schema V1 — frontend mirror of src-tauri/src/plugins/payload.rs.
// The backend validates again in start_execution (before a worker starts) and in the
// worker; this copy lets the dispatcher and the Plugin Manager refuse a bad payload
// early. Invariant: no field carries a path, file name, text, password or content.

export const PLUGIN_HOOK_PAYLOAD_SCHEMA_VERSION = 1;

// PluginExportFormatV1 (lowercase serde names). pluginHookPayloads.test.js checks the
// list against the Rust enum.
export const PLUGIN_EXPORT_FORMATS = Object.freeze([
  "pdf", "docx", "pptx", "html", "rtf", "md", "txt", "png", "jpeg", "svg", "webp",
]);

const FIELD_SCHEMA_VERSION = Object.freeze({
  name: "schema_version", type: "integer", min: PLUGIN_HOOK_PAYLOAD_SCHEMA_VERSION,
  max: PLUGIN_HOOK_PAYLOAD_SCHEMA_VERSION, default: PLUGIN_HOOK_PAYLOAD_SCHEMA_VERSION,
  label: "schema_version",
});
const FIELD_PAGE_COUNT = Object.freeze({
  name: "page_count", type: "integer", min: 1, max: 0xffffffff, default: 1, label: "page_count (sayfa sayısı)",
});
const FIELD_FORMAT = Object.freeze({
  name: "format", type: "enum", values: PLUGIN_EXPORT_FORMATS, default: "pdf", label: "format",
});

export const PLUGIN_HOOK_PAYLOAD_SCHEMAS = Object.freeze({
  on_document_open: Object.freeze({ fields: Object.freeze([FIELD_SCHEMA_VERSION, FIELD_PAGE_COUNT]) }),
  on_export_complete: Object.freeze({ fields: Object.freeze([FIELD_SCHEMA_VERSION, FIELD_FORMAT, FIELD_PAGE_COUNT]) }),
  on_custom_command: Object.freeze({ fields: Object.freeze([FIELD_SCHEMA_VERSION]) }),
});

export const PLUGIN_HOOK_NAMES = Object.freeze(Object.keys(PLUGIN_HOOK_PAYLOAD_SCHEMAS));

/** Default (schema-valid) payload for `hookName`; null for an unknown hook. */
export function defaultHookPayload(hookName) {
  const schema = PLUGIN_HOOK_PAYLOAD_SCHEMAS[hookName];
  if (!schema) return null;
  return Object.fromEntries(schema.fields.map((field) => [field.name, field.default]));
}

function fieldError(field, value) {
  if (value === undefined) return `'${field.name}' alanı eksik.`;
  if (field.type === "integer") {
    if (typeof value !== "number" || !Number.isInteger(value)) return `'${field.name}' bir tam sayı olmalı.`;
    if (value < field.min || value > field.max) {
      return field.min === field.max
        ? `'${field.name}' yalnız ${field.min} olabilir.`
        : `'${field.name}' ${field.min} ile ${field.max} arasında olmalı.`;
    }
    return null;
  }
  if (field.type === "enum") {
    if (typeof value !== "string" || !field.values.includes(value)) {
      return `'${field.name}' şunlardan biri olmalı: ${field.values.join(", ")}.`;
    }
    return null;
  }
  return `'${field.name}' alanının türü bilinmiyor.`;
}

/**
 * Checks `payload` against the V1 schema of `hookName`: every field present with the
 * right type, no other field. Returns `{ ok: true, payload }` (a clean copy) or
 * `{ ok: false, error }`.
 */
export function validateHookPayload(hookName, payload) {
  const schema = PLUGIN_HOOK_PAYLOAD_SCHEMAS[hookName];
  if (!schema) return { ok: false, error: `Bilinmeyen kanca: '${hookName}'.` };
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    return { ok: false, error: "Kanca yükü bir nesne olmalı." };
  }
  const allowed = new Set(schema.fields.map((field) => field.name));
  const unknown = Object.keys(payload).filter((key) => !allowed.has(key));
  if (unknown.length > 0) return { ok: false, error: `Şemada olmayan alan: ${unknown.join(", ")}.` };
  for (const field of schema.fields) {
    const error = fieldError(field, payload[field.name]);
    if (error) return { ok: false, error };
  }
  return { ok: true, payload: Object.fromEntries(schema.fields.map((field) => [field.name, payload[field.name]])) };
}

export function documentOpenPayload(pageCount) {
  return { schema_version: PLUGIN_HOOK_PAYLOAD_SCHEMA_VERSION, page_count: pageCount };
}

export function exportCompletePayload(format, pageCount) {
  return { schema_version: PLUGIN_HOOK_PAYLOAD_SCHEMA_VERSION, format, page_count: pageCount };
}

export function customCommandPayload() {
  return { schema_version: PLUGIN_HOOK_PAYLOAD_SCHEMA_VERSION };
}

const EXTENSION_FORMATS = Object.freeze({ jpg: "jpeg", markdown: "md" });

/** Maps an output extension or format id ("jpg", "PNG", "markdown") to a V1 format, or null. */
export function exportFormatFromExtension(value) {
  const key = String(value ?? "").trim().toLowerCase().replace(/^\./, "");
  const format = EXTENSION_FORMATS[key] ?? key;
  return PLUGIN_EXPORT_FORMATS.includes(format) ? format : null;
}
