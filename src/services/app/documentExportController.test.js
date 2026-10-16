import { describe, expect, it, vi } from "vitest";
import { createDocumentExportController } from "./documentExportController.js";

function setup(overrides = {}) {
  const context = { isPdf: true, sessionId: "session-a", contentGeneration: 5, imagePath: "", pageIndex: 1, displayPageIndex: 1, pageOrder: [0, 1, 2], fileName: "report.pdf", rotation: 0, flipH: false, flipV: false };
  const extractPages = vi.fn(async () => undefined);
  const controller = createDocumentExportController({
    getContext: () => ({ ...context }), saveFile: vi.fn(async () => "/tmp/out.pdf"), requestInput: vi.fn(async () => "1-2, 3"),
    exportPage: vi.fn(async () => undefined), extractPages, convertImage: vi.fn(async () => undefined),
    onToast: vi.fn(), onError: vi.fn(), ...overrides,
  });
  return { controller, context, extractPages };
}

describe("documentExportController", () => {
  it("parses a split request and exports the captured PDF page indices", async () => {
    const { controller, extractPages } = setup();
    await controller.splitPdf();
    expect(extractPages).toHaveBeenCalledWith("session-a", 5, [0, 1, 2], "/tmp/out.pdf");
  });

  it("does not report a stale page-export completion", async () => {
    const context = { isPdf: true, sessionId: "session-a", contentGeneration: 5, imagePath: "", pageIndex: 1, displayPageIndex: 1, pageOrder: [0, 1], fileName: "report.pdf", rotation: 0, flipH: false, flipV: false };
    const onToast = vi.fn();
    const controller = createDocumentExportController({
      getContext: () => ({ ...context }), saveFile: vi.fn(async () => "/tmp/page.png"), requestInput: vi.fn(),
      exportPage: vi.fn(async () => { context.contentGeneration += 1; }), extractPages: vi.fn(), convertImage: vi.fn(),
      onToast, onError: vi.fn(),
    });
    await controller.exportCurrentPageImage();
    expect(onToast).not.toHaveBeenCalled();
  });

  it("exports the physical page while labelling it with its displayed number", async () => {
    const exportPage = vi.fn(async () => undefined);
    const saveFile = vi.fn(async () => "/tmp/page.png");
    const { controller, context } = setup({ exportPage, saveFile });
    context.pageIndex = 2;
    context.displayPageIndex = 0;
    await controller.exportCurrentPageImage();
    expect(saveFile).toHaveBeenCalledWith(expect.objectContaining({ defaultPath: "report_sayfa_1.svg" }));
    expect(exportPage).toHaveBeenCalledWith("session-a", 5, 2, "/tmp/page.png", "png");
  });

  it("maps typed (displayed) page numbers to physical pages when splitting", async () => {
    const extractPages = vi.fn(async () => undefined);
    const { controller, context } = setup({ extractPages, requestInput: vi.fn(async () => "1, 3") });
    context.pageOrder = [2, 0, 1];
    await controller.splitPdf();
    expect(extractPages).toHaveBeenCalledWith("session-a", 5, [2, 1], "/tmp/out.pdf");
  });

  it("keeps the typed order when extracting", async () => {
    const extractPages = vi.fn(async () => undefined);
    const { controller } = setup({ extractPages, requestInput: vi.fn(async () => "3, 1-2") });
    await controller.splitPdf();
    expect(extractPages).toHaveBeenCalledWith("session-a", 5, [2, 0, 1], "/tmp/out.pdf");
  });

  it.each([
    ["5", '"5" sınır dışında'],
    ["0", '"0" sınır dışında'],
    ["3-1", '"3-1" ters aralık'],
    ["1-2-3", '"1-2-3" geçersiz'],
    ["2.5", '"2.5" geçersiz'],
    ["1,,2", "2. parça boş"],
    ["1-2, 2", '"2" sayfa 2 tekrar ediyor'],
  ])("rejects %j with the offending part instead of dropping it", async (input, message) => {
    const extractPages = vi.fn(async () => undefined);
    const saveFile = vi.fn(async () => "/tmp/out.pdf");
    const onToast = vi.fn();
    const { controller } = setup({ extractPages, saveFile, onToast, requestInput: vi.fn(async () => input) });
    await controller.splitPdf();
    expect(onToast).toHaveBeenCalledWith(expect.stringContaining(message));
    expect(saveFile).not.toHaveBeenCalled();
    expect(extractPages).not.toHaveBeenCalled();
  });
});
