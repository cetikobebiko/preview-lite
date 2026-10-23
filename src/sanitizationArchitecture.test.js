import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { getFeature } from "./featureRegistry.js";

const read = (path) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("8C.3 private sanitization transform architecture boundary", () => {
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

  it("runs independent verification in a second private worker without publication", () => {
    const worker = read("src-tauri/src/job_worker.rs");
    const verifier = read("src-tauri/src/sanitization_verifier.rs");
    const orchestration = read("src-tauri/src/sanitization_transform.rs");
    const contracts = read("src-tauri/src/sanitization.rs");
    const lib = read("src-tauri/src/lib.rs");

    expect(worker).toContain("VerifySanitizedPdf");
    expect(worker).toContain("SanitizationVerification");
    expect(verifier).toContain("Document::load(&artifact)");
    expect(verifier).toContain("inspect_snapshot(");
    expect(verifier).toContain("open_pdfium_doc");
    expect(verifier).toContain("observed_payload_digests");
    expect(verifier).toContain("verify_full_rewrite");
    expect(verifier).not.toMatch(/target_path|publication|atomic_publish|save dialog/i);
    expect(orchestration).toContain("transform_and_verify_document_sanitization_core");
    expect(orchestration).toContain("SANITIZATION_TRANSFORM_ARTIFACT_DIGEST_MISMATCH");
    expect(orchestration).toContain("validate_verification_report_for_transform");
    expect(contracts).toContain("SanitizationVerificationReport");
    expect(contracts).toContain("SkippedWithReason");
    expect(lib).not.toMatch(/generate_handler!\[[\s\S]*VerifySanitizedPdf/);
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
    // lopdf 0.45 needs the snapshot's bytes (raw trailer, password at load time).
    expect(scanner).toContain("std::fs::read(snapshot_path)");
    expect(scanner).toContain("lopdf_compat::load_mem_with_credential(&bytes, credential)");
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
    const inspectionWrapper = wrapper.match(/export function startSanitizationInspection[\s\S]*?\n}/)?.[0] ?? "";
    expect(inspectionWrapper).not.toMatch(/_sanitized\.pdf|outputPath|targetPath|save\(/);
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

  it("keeps transform private, snapshot-bound, fixed-name and unpublished", () => {
    const transform = read("src-tauri/src/sanitization_transform.rs");
    const worker = read("src-tauri/src/job_worker.rs");
    const lib = read("src-tauri/src/lib.rs");
    const contracts = read("src-tauri/src/sanitization.rs");

    expect(worker).toContain("SanitizePdf");
    expect(worker).toContain("SanitizationTransform");
    expect(transform).toContain("std::fs::read(snapshot_path)");
    expect(transform).toContain("Document::load_mem(&input)");
    expect(contracts).toContain('SANITIZATION_TRANSFORM_OUTPUT_NAME: &str = "sanitized-transform.pdf"');
    expect(transform).toContain("expected_inspection_digest");
    expect(transform).toContain("MAX_SANITIZATION_DECODED_STREAM_BYTES");
    expect(transform).toContain("validate_transform_report_for_policy");
    expect(transform).toContain("validate_transform_worker_result");
    expect(transform).toContain("validate_output_file");
    expect(transform).toContain("sha256_file(&artifact)");
    expect(transform).toContain('status: "transformed_unverified"');
    expect(transform).not.toMatch(/target_path|output_path: String|atomic_publish|save dialog/i);
    expect(lib).not.toMatch(/generate_handler!\[[\s\S]*transform_document_sanitization_core/);
  });

  it("binds reports to the original request and keeps artifacts in one job lifecycle", () => {
    const contracts = read("src-tauri/src/sanitization.rs");
    const transform = read("src-tauri/src/sanitization_transform.rs");
    const lifecycle = read("docs/engineering/pdf-sanitization-threat-model.md");

    expect(contracts).toContain("validate_transform_report_for_policy");
    expect(contracts).toContain("requested != reported");
    expect(transform).toContain("validate_transform_report_for_policy(");
    expect(lifecycle).toContain("transform worker → parent report/artifact validation");
    expect(lifecycle).toContain("terminal cleanup job");
  });

  it("removes the broad boolean model, implicit target, and source auto-replacement from production frontend", () => {
    const panel = read("src/components/SanitizationPanel.svelte");
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
      expect(panel).not.toContain(legacyControl);
      expect(contracts).not.toContain(legacyControl);
    }
    expect(controller).not.toMatch(/_sanitized\.pdf|sanitizeDocument/);
    expect(runtime).not.toContain("sanitize: transform.sanitizeDocument");
    expect(app).not.toMatch(/onSanitize|featureRuntime\.sanitize/);
    expect(panel).not.toMatch(/auto.?open|openDocument|active document/i);
  });

  it("keeps canonical execution behind controller/runtime and strict versioned IPC", () => {
    const component = read("src/components/SanitizationPanel.svelte");
    const controller = read("src/services/app/sanitizationController.js");
    const wrapper = read("src/services/backend/sanitization.js");
    const contracts = read("src/services/backend/contracts.js");
    const runtime = read("src/services/app/featureRuntime.js");
    const app = read("src/App.svelte");
    const lib = read("src-tauri/src/lib.rs");

    expect(component).not.toMatch(/callBackend|invoke\(|startSanitizationExecution/);
    expect(controller).toContain('from "../backend/sanitization.js"');
    expect(controller).toContain("startSanitizationExecution");
    expect(wrapper).toContain('callBackend("executeDocumentSanitization"');
    expect(contracts).toMatch(/executeDocumentSanitization: command\("execute_document_sanitization"/);
    const executionContract = contracts.match(/executeDocumentSanitization: command\("execute_document_sanitization", \{[\s\S]*?\}\),/)?.[0] ?? "";
    expect(executionContract).toContain("sessionId");
    expect(executionContract).toContain("expectedContentGeneration");
    expect(executionContract).toContain("policy");
    expect(executionContract).toContain("targetPath");
    expect(executionContract).not.toMatch(/inputPath|workingPath|snapshotPath|options/);
    expect(runtime).toContain("const sanitization = Object.freeze");
    expect(app).not.toMatch(/callBackend|startSanitizationExecution|save\(/);
    expect(lib).toContain("sanitization_publication::execute_document_sanitization");
  });
});
