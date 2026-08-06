import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const appSource = readFileSync(resolve(process.cwd(), "src/App.svelte"), "utf8");
const shareControllerSource = readFileSync(resolve(process.cwd(), "src/services/app/shareActionController.js"), "utf8");
const ocrControllerSource = readFileSync(resolve(process.cwd(), "src/services/app/ocrActionController.js"), "utf8");
const documentRuntimeSource = readFileSync(resolve(process.cwd(), "src/services/app/documentRuntime.js"), "utf8");
const viewerRuntimeSource = readFileSync(resolve(process.cwd(), "src/services/app/viewerRuntime.js"), "utf8");
const featureRuntimeSource = readFileSync(resolve(process.cwd(), "src/services/app/featureRuntime.js"), "utf8");

describe("App state consolidation & stale guard rules", () => {
  it("does not declare local shadow session variables in App.svelte", () => {
    expect(appSource).not.toMatch(/let\s+activeSessionId\b/);
    expect(appSource).not.toMatch(/let\s+activeSessionSnapshot\b/);
    expect(appSource).not.toMatch(/let\s+documentGenerationId\b/);
    expect(appSource).not.toMatch(/let\s+selectedFilePath\b/);
  });

  it("derives isDirty strictly from documentSession.dirty", () => {
    expect(documentRuntimeSource).toContain("get isDirty() { return documentSession.dirty; }");
    expect(appSource).toContain("isDirty={documentRuntime.isDirty}");
  });

  it("derives isPdf strictly from Boolean(documentSession.active)", () => {
    expect(documentRuntimeSource).toContain("const isPdf = () => Boolean(documentSession.active);");
    expect(appSource).toContain("isPdf={documentRuntime.isPdf}");
  });

  it("delegates render stale checks to the viewer controller", () => {
    expect(appSource).toContain("createViewerRuntime({");
    expect(viewerRuntimeSource).toMatch(/createViewerController\(\{/);
    expect(appSource).not.toMatch(/evaluateRenderGuard\(/);
  });

  it("delegates prepress ownership to the dedicated state factory", () => {
    expect(appSource).toMatch(/const prepress = createPrepressState\(\);/);
    expect(appSource).toMatch(/prepress\.updateContext\(getPrepressContext\(\)\)/);
    expect(appSource).toContain('applyPrepress("applySeparations", event)');
    expect(appSource).toMatch(/prepress\.payload\(state, pageIndex, getPrepressContext\(\), payloadKey\)/);
  });

  it("maintains separate image state without storing image paths in documentSession store", () => {
    expect(appSource).toMatch(/const viewer = createViewerState\(\);/);
    expect(appSource).toMatch(/viewer\.activeImageSourcePath/);
    expect(appSource).not.toMatch(/documentSession\.activeImageSourcePath/);
  });

  it("reads viewer presentation derivations from the authoritative viewer store", () => {
    expect(appSource).toContain("spreads={viewer.spreads}");
    expect(appSource).toContain("physicalPageIndex={viewer.physicalPageIndex}");
    expect(appSource).not.toMatch(/let\s+spreads\s*=\s*\$derived/);
    expect(appSource).not.toMatch(/let\s+physicalPageIndex\s*=\s*\$derived/);
    expect(appSource).not.toContain("function resolvePhysicalPageIndex");
  });

  it("delegates transient presentation ownership to uiState and its controller", () => {
    expect(appSource).toContain("createPresentationController({ ui })");
    expect(appSource).toContain("toastMsg={ui.toast}");
    expect(appSource).toContain("errorMsg={ui.error}");
    expect(appSource).not.toMatch(/let\s+(toastMsg|errorMsg|passwordModalError)\s*=\s*\$state/);
  });

  it("separates active-PDF sharing from external image/file sharing", () => {
    expect(featureRuntimeSource).toContain("createShareActionController");
    expect(shareControllerSource).toContain("context.sessionId, context.contentGeneration");
    expect(shareControllerSource).toContain("context.imagePath");
  });

  it("keeps image OCR unsupported and sends PDF OCR through session identity", () => {
    expect(featureRuntimeSource).toContain("createOcrActionController");
    expect(ocrControllerSource).toContain("if (!context.isPdf || !context.sessionId)");
    expect(ocrControllerSource).toContain("runOcr(sessionId, contentGeneration, pageIndex, featureContext)");
    expect(ocrControllerSource).not.toMatch(/runOcr\(.*imagePath/);
  });
});
