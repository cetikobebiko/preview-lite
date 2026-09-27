import { describe, expect, it } from "vitest";
import { readFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";
import { getFeature, getFeaturePresentation } from "./featureRegistry.js";

const root = process.cwd();
const read = (path) => readFileSync(resolve(root, path), "utf8");

describe("basic accessibility audit architecture", () => {
  it("keeps public naming and feature states honest", () => {
    expect(getFeature("basic_accessibility_audit")).toMatchObject({ status: "experimental", enabled: true });
    expect(getFeature("pdf_ua_validation")).toBeNull();
    expect(getFeature("pdf_ua_machine_validation")).toMatchObject({ status: "disabled", enabled: false });
    expect(getFeature("pdf_auto_tag")).toMatchObject({ status: "disabled", enabled: false });
    expect(getFeature("reading_order_editing")).toMatchObject({ status: "disabled", enabled: false });
    for (const id of ["pdf_ua_machine_validation", "pdf_auto_tag", "reading_order_editing"]) {
      expect(getFeaturePresentation(id, { platform: "macos", build: "development" }).runnable).toBe(false);
    }
  });

  it("removes misleading production commands and editing affordances", () => {
    const a11y = read("src-tauri/src/a11y.rs");
    const session = read("src-tauri/src/document_session.rs");
    const ipc = read("src-tauri/src/worker/ipc.rs");
    const panel = read("src/components/A11yPanel.svelte");
    const tree = read("src/components/TagTreeExplorer.svelte");
    const workspacePanels = read("src/components/WorkspacePanels.svelte");
    expect(a11y).not.toMatch(/fn update_alt_text|fn update_reading_order|struct AccessibilityReport/);
    expect(session).not.toMatch(/fn validate_session_pdf_ua|fn update_session_alt_text/);
    expect(ipc).not.toMatch(/ValidatePdfUa|PdfUaValidation/);
    expect(panel).not.toMatch(/report\.score|\/100|Screen Reader Preview|Save Changes|Auto-Tag/);
    expect(tree).not.toMatch(/draggable|dragstart|dragover|drop|nodeReorder|canReorder/);
    expect(existsSync(resolve(root, "src/components/ReadingOrderOverlay.svelte"))).toBe(false);
    expect(workspacePanels).toMatch(/ui\.panels\.a11y && isPdf/);
  });

  it("keeps the audit session-native, worker-owned, bounded and read-only", () => {
    const session = read("src-tauri/src/document_session.rs");
    const worker = read("src-tauri/src/worker/pdfium.rs");
    const audit = read("src-tauri/src/a11y.rs");
    const start = session.indexOf("pub async fn audit_session_accessibility");
    const body = session.slice(start, start + 1800);
    expect(start).toBeGreaterThanOrEqual(0);
    expect(body).toMatch(/acquire_worker_read/);
    expect(body).toMatch(/WorkerCommand::AuditAccessibility/);
    expect(body).not.toMatch(/Document::load|mutate|save\(/);
    expect(worker).toMatch(/document\.accessibility\.as_ref/);
    expect(audit).toMatch(/MAX_STRUCTURE_DEPTH|MAX_STRUCTURE_NODES|MAX_ROLE_MAP_ENTRIES|MAX_PARENT_TREE_ENTRIES|MAX_REPORT_BYTES/);
    expect(audit).toMatch(/HashSet<ObjectId>/);
  });
});
