import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  PLUGIN_EXPORT_FORMATS,
  PLUGIN_HOOK_NAMES,
  defaultHookPayload,
  exportFormatFromExtension,
  validateHookPayload,
} from "./pluginHookPayloads.js";

const payloadSource = readFileSync(resolve(process.cwd(), "src-tauri/src/plugins/payload.rs"), "utf8");
const manifestSource = readFileSync(resolve(process.cwd(), "src-tauri/src/plugins/manifest.rs"), "utf8");

describe("plugin hook payload schema V1 (frontend mirror)", () => {
  it("lists the same export formats as PluginExportFormatV1", () => {
    const rust = [...payloadSource.matchAll(/Self::\w+ => "([a-z]+)",/g)].map((m) => m[1]);
    expect(rust.length).toBeGreaterThan(0);
    expect([...PLUGIN_EXPORT_FORMATS].sort()).toEqual([...new Set(rust)].sort());
  });

  it("has a schema for exactly the hooks PluginHook declares", () => {
    const rustHooks = [...manifestSource.matchAll(/Self::\w+ => "(on_[a-z_]+)"/g)].map((m) => m[1]);
    expect([...PLUGIN_HOOK_NAMES].sort()).toEqual([...new Set(rustHooks)].sort());
  });

  it("generates a valid default payload for every hook", () => {
    expect(defaultHookPayload("on_document_open")).toEqual({ schema_version: 1, page_count: 1 });
    expect(defaultHookPayload("on_export_complete")).toEqual({ schema_version: 1, format: "pdf", page_count: 1 });
    expect(defaultHookPayload("on_custom_command")).toEqual({ schema_version: 1 });
    for (const hook of PLUGIN_HOOK_NAMES) expect(validateHookPayload(hook, defaultHookPayload(hook)).ok).toBe(true);
    expect(defaultHookPayload("on_anything")).toBeNull();
  });

  it("rejects unknown fields, other hooks' payloads and wrong schema_version", () => {
    expect(validateHookPayload("on_document_open", { schema_version: 1, page_count: 2, file_name: "a.pdf" }).ok).toBe(false);
    expect(validateHookPayload("on_custom_command", { schema_version: 1, page_count: 2 }).ok).toBe(false);
    expect(validateHookPayload("on_document_open", { schema_version: 1 }).ok).toBe(false);
    expect(validateHookPayload("on_export_complete", { schema_version: 1, page_count: 2 }).ok).toBe(false);
    expect(validateHookPayload("on_custom_command", { schema_version: 2 }).ok).toBe(false);
    expect(validateHookPayload("on_document_open", { schema_version: 1, page_count: 0 }).ok).toBe(false);
    expect(validateHookPayload("on_document_open", { schema_version: 1, page_count: 1.5 }).ok).toBe(false);
    expect(validateHookPayload("on_export_complete", { schema_version: 1, format: "exe", page_count: 1 }).ok).toBe(false);
    expect(validateHookPayload("on_custom_command", null).ok).toBe(false);
    expect(validateHookPayload("on_custom_command", []).ok).toBe(false);
    expect(validateHookPayload("on_nothing", {}).ok).toBe(false);
  });

  it("maps output extensions to V1 formats", () => {
    expect(exportFormatFromExtension("jpg")).toBe("jpeg");
    expect(exportFormatFromExtension("PNG")).toBe("png");
    expect(exportFormatFromExtension("markdown")).toBe("md");
    expect(exportFormatFromExtension(".svg")).toBe("svg");
    expect(exportFormatFromExtension("xlsx")).toBeNull();
    expect(exportFormatFromExtension(undefined)).toBeNull();
  });
});
