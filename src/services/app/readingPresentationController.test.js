import { describe, expect, it, vi } from "vitest";
import { createReadingPresentationController } from "./readingPresentationController.js";

function setup() {
  const viewer = { documentTextContext: "", documentTextPageIndex: -1, bionicText: "", bionicSource: null };
  const ui = { isBionicModalOpen: false };
  // Physical page 4 is shown at display position 1.
  const context = { isPdf: true, sessionId: "session-a", contentGeneration: 2, pageIndex: 4, displayPageIndex: 1, viewer, ui };
  const extractText = vi.fn(async () => "\n--- Sayfa 5 ---\n\nSayfanın PDF metni");
  const performOcr = vi.fn(async () => "OCR metni");
  const controller = createReadingPresentationController({
    getContext: () => ({ ...context }), extractText, performOcr, featureContext: {}, onToast: vi.fn(),
  });
  return { controller, context, viewer, ui, extractText, performOcr };
}

describe("readingPresentationController", () => {
  it("shows the current physical page's PDF text and labels it as that single page", async () => {
    const { controller, viewer, ui, extractText, performOcr } = setup();
    await controller.openBionicReading();
    expect(extractText).toHaveBeenCalledWith("session-a", 2, [4], "text");
    expect(performOcr).not.toHaveBeenCalled();
    expect(viewer.bionicText).toBe("Sayfanın PDF metni");
    expect(viewer.bionicSource).toEqual({ pageNumber: 2, method: "pdf" });
    expect(ui.isBionicModalOpen).toBe(true);
  });

  it("does not reuse OCR text recognized from another page", async () => {
    const { controller, viewer, extractText, performOcr } = setup();
    extractText.mockResolvedValue("\n--- Sayfa 5 ---\n\n  ");
    viewer.documentTextContext = "Sayfa 1'in OCR metni";
    viewer.documentTextPageIndex = 0;
    await controller.openBionicReading();
    expect(performOcr).toHaveBeenCalledWith("session-a", 2, 4, {});
    expect(viewer.bionicText).toBe("OCR metni");
    expect(viewer.bionicSource).toEqual({ pageNumber: 2, method: "ocr" });
  });

  it("reuses OCR text already recognized from the same scanned page", async () => {
    const { controller, viewer, extractText, performOcr } = setup();
    extractText.mockResolvedValue("");
    viewer.documentTextContext = "Bu sayfanın OCR metni";
    viewer.documentTextPageIndex = 4;
    await controller.openBionicReading();
    expect(performOcr).not.toHaveBeenCalled();
    expect(viewer.bionicText).toBe("Bu sayfanın OCR metni");
  });

  it("does not open the modal when the text belongs to an older generation", async () => {
    const { controller, context, viewer, ui, extractText } = setup();
    extractText.mockImplementationOnce(async () => { context.contentGeneration += 1; return "Eski metin"; });
    await controller.openBionicReading();
    expect(viewer.bionicText).toBe("");
    expect(ui.isBionicModalOpen).toBe(false);
  });

  it("refuses standalone images without running extraction or OCR", async () => {
    const { controller, context, extractText, performOcr, ui } = setup();
    context.isPdf = false;
    context.sessionId = null;
    await controller.openBionicReading();
    expect(extractText).not.toHaveBeenCalled();
    expect(performOcr).not.toHaveBeenCalled();
    expect(ui.isBionicModalOpen).toBe(false);
  });
});
