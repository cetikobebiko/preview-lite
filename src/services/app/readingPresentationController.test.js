import { describe, expect, it, vi } from "vitest";
import { createReadingPresentationController } from "./readingPresentationController.js";

function setup() {
  const viewer = { documentTextContext: "Hazır metin", bionicText: "" };
  const ui = { isBionicModalOpen: false };
  const context = { isPdf: true, sessionId: "session-a", contentGeneration: 2, pageIndex: 4, viewer, ui };
  const performOcr = vi.fn(async () => "OCR metni");
  const controller = createReadingPresentationController({
    getContext: () => ({ ...context }), performOcr, featureContext: {}, onToast: vi.fn(),
  });
  return { controller, context, viewer, ui, performOcr };
}

describe("readingPresentationController", () => {
  it("opens Bionic Reading from existing viewer text without another state copy", async () => {
    const { controller, viewer, ui, performOcr } = setup();
    await controller.openBionicReading();
    expect(performOcr).not.toHaveBeenCalled();
    expect(viewer.bionicText).toBe("Hazır metin");
    expect(ui.isBionicModalOpen).toBe(true);
  });

  it("does not open the modal when an OCR result belongs to an older generation", async () => {
    const { controller, context, viewer, ui, performOcr } = setup();
    viewer.documentTextContext = "";
    performOcr.mockImplementationOnce(async () => { context.contentGeneration += 1; return "Eski metin"; });
    await controller.openBionicReading();
    expect(viewer.bionicText).toBe("");
    expect(ui.isBionicModalOpen).toBe(false);
  });
});
