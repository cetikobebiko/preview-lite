import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/svelte";
import AppOverlayHost from "./AppOverlayHost.svelte";

afterEach(cleanup);

function renderOverlay(toastMsg) {
  return render(AppOverlayHost, {
    ui: { isDraggingFileOver: false, activeModal: null },
    toastMsg,
    onPasswordSubmit: vi.fn(),
    onPasswordCancel: vi.fn(),
    asyncPrompt: { active: null, submit: vi.fn(), cancel: vi.fn() },
  });
}

describe("AppOverlayHost toast", () => {
  it("uppercases Turkish toast text with Turkish casing rules", () => {
    const view = renderOverlay("Sürüklenen dosya açıldı: rapor.pdf. Diğer 2 dosya yok sayıldı.");

    expect(view.getByText("// SÜRÜKLENEN DOSYA AÇILDI: RAPOR.PDF. DİĞER 2 DOSYA YOK SAYILDI.")).toBeTruthy();
  });

  it("keeps dotted and dotless i distinct", () => {
    const view = renderOverlay("ilk ışık");

    expect(view.getByText("// İLK IŞIK")).toBeTruthy();
  });
});
