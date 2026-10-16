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
  it("shows the toast text as written, as a status message", () => {
    const view = renderOverlay("Sürüklenen dosya açıldı: rapor.pdf. Diğer 2 dosya yok sayıldı.");

    const toast = view.getByRole("status");
    expect(toast.textContent.trim()).toBe("Sürüklenen dosya açıldı: rapor.pdf. Diğer 2 dosya yok sayıldı.");
  });

  it("renders nothing without a message", () => {
    const view = renderOverlay("");
    expect(view.queryByRole("status")).toBeNull();
  });
});
