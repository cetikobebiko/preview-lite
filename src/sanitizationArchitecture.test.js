import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { getFeature } from "./featureRegistry.js";

const read = (path) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("8C.2 sanitization inspection architecture boundary", () => {
  it("keeps sanitization disabled and fail-closed", () => {
    const feature = getFeature("sanitization");
    expect(feature.status).toBe("disabled");
    expect(feature.enabled).toBe(false);

    const backend = read("src-tauri/src/redaction.rs");
    expect(backend).toMatch(/fn sanitize_pdf_document_unavailable\s*\(\s*\)/);
    expect(backend).toMatch(/"FEATURE_UNAVAILABLE"/);
    expect(backend).toMatch(
      /pub async fn sanitize_pdf_document[\s\S]*?sanitize_pdf_document_unavailable\(\)/,
    );
  });

  it("keeps parsing in the ephemeral snapshot worker and exposes no path/output IPC", () => {
    const command = read("src-tauri/src/sanitization.rs");
    const scanner = read("src-tauri/src/sanitization_inspection.rs");
    const worker = read("src-tauri/src/job_worker.rs");
    const contracts = read("src/services/backend/contracts.js");
    const wrapper = read("src/services/backend/sanitization.js");

    expect(command).toContain("run_snapshot_job");
    expect(command).toContain("JobCommand::InspectDocumentSanitization");
    expect(command).not.toMatch(/Document::load|get_pdfium|open_pdfium_doc|lopdf::/);
    expect(scanner).toContain("Document::load(snapshot_path)");
    expect(worker).toContain("InspectDocumentSanitization");
    expect(worker).toContain("SanitizationInspection");
    expect(contracts).toMatch(/inspectDocumentSanitization: command\("inspect_document_sanitization"/);
    const inspectionContract = contracts.match(/inspectDocumentSanitization: command\("inspect_document_sanitization", \{[\s\S]*?\}\),/)?.[0] ?? "";
    expect(inspectionContract).toContain("jobId");
    expect(inspectionContract).toContain("sessionId");
    expect(inspectionContract).toContain("expectedContentGeneration");
    expect(inspectionContract).toContain("policySchemaVersion");
    expect(inspectionContract).not.toMatch(/inputPath|workingPath|snapshotPath|outputPath|targetPath|password|options/);
    expect(wrapper).toContain("runSessionJob");
    expect(wrapper).not.toMatch(/_sanitized\.pdf|outputPath|targetPath|save\(/);
  });

  it("keeps inspection read-only and lifecycle ownership outside App", () => {
    const scanner = read("src-tauri/src/sanitization_inspection.rs");
    const scannerProduction = scanner.split("#[cfg(test)]")[0];
    const controller = read("src/services/app/sanitizationInspectionController.js");
    const app = read("src/App.svelte");
    const runtime = read("src/services/app/featureRuntime.js");
    expect(scannerProduction).not.toMatch(/delete_object|prune_objects|catalog_mut|save_to|\.save\(/);
    expect(controller).toContain('from "../backend/sanitization.js"');
    expect(controller).toContain("captured.contentGeneration");
    expect(controller).not.toMatch(/_sanitized\.pdf|outputPath|targetPath|save\(/);
    expect(app).not.toMatch(/inspectDocumentSanitization|startSanitizationInspection|callBackend/);
    expect(runtime).toContain('runUiAction("sanitization"');
  });

  it("records the broad booleans and implicit output path as dormant migration debt", () => {
    const dialog = read("src/components/SanitizationDialog.svelte");
    const contracts = read("src/services/backend/contracts.js");
    const controller = read("src/services/app/documentTransformController.js");
    const runtime = read("src/services/app/featureRuntime.js");
    const app = read("src/App.svelte");

    for (const legacyControl of [
      "remove_metadata",
      "remove_annotations",
      "remove_javascript",
      "remove_embedded_files",
      "remove_hidden_layers",
    ]) {
      expect(dialog).toContain(legacyControl);
      expect(contracts).toContain(legacyControl);
    }
    expect(controller).toContain('context.path.replace(".pdf", "_sanitized.pdf")');
    expect(runtime).toContain("sanitize: transform.sanitizeDocument");
    expect(app).toContain("onSanitize={(event) => featureRuntime.sanitize(event.detail)}");
  });
});
