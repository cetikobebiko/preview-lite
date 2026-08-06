import { describe, expect, it, vi } from "vitest";
import { createDocumentExportController } from "./documentExportController.js";

function setup(overrides = {}) {
  const context = { isPdf: true, isLoadedPdfDocument: true, sessionId: "session-a", contentGeneration: 5, imagePath: "", pageIndex: 1, pageOrder: [0, 1, 2], fileName: "report.pdf", annotations: [{ tool_type: "pen" }], rotation: 0, flipH: false, flipV: false };
  const guard = vi.fn(async (_feature, _runtime, action) => action());
  const extractPages = vi.fn(async () => undefined);
  const controller = createDocumentExportController({
    getContext: () => ({ ...context }), saveFile: vi.fn(async () => "/tmp/out.pdf"), requestInput: vi.fn(async () => "1-2, 3"),
    exportPage: vi.fn(async () => undefined), extractPages, convertImage: vi.fn(async () => undefined), burnAnnotations: vi.fn(async () => undefined),
    guard, featureContext: {}, reportUnavailable: () => false, onToast: vi.fn(), onError: vi.fn(), ...overrides,
  });
  return { controller, context, guard, extractPages };
}

describe("documentExportController", () => {
  it("parses a split request and exports the captured PDF page indices", async () => {
    const { controller, extractPages } = setup();
    await controller.splitPdf();
    expect(extractPages).toHaveBeenCalledWith("session-a", 5, [0, 1, 2], "/tmp/out.pdf");
  });

  it("uses one guarded transaction for burning captured annotations", async () => {
    const { controller, guard } = setup();
    await controller.burnInPdf();
    expect(guard).toHaveBeenCalledWith("annotation_burn_export", {}, expect.any(Function));
  });

  it("does not report a stale page-export completion", async () => {
    const context = { isPdf: true, isLoadedPdfDocument: true, sessionId: "session-a", contentGeneration: 5, imagePath: "", pageIndex: 1, pageOrder: [0, 1], fileName: "report.pdf", annotations: [], rotation: 0, flipH: false, flipV: false };
    const onToast = vi.fn();
    const controller = createDocumentExportController({
      getContext: () => ({ ...context }), saveFile: vi.fn(async () => "/tmp/page.png"), requestInput: vi.fn(),
      exportPage: vi.fn(async () => { context.contentGeneration += 1; }), extractPages: vi.fn(), convertImage: vi.fn(), burnAnnotations: vi.fn(),
      guard: vi.fn(), featureContext: {}, reportUnavailable: () => false, onToast, onError: vi.fn(),
    });
    await controller.exportCurrentPageImage();
    expect(onToast).not.toHaveBeenCalled();
  });
});
