import { afterEach, describe, expect, it, vi } from "vitest";
import { createPresentationController } from "./presentationController.js";

function createUi() {
  return {
    activeModal: null,
    toast: "",
    error: "",
    openModal(type, props) { this.activeModal = { type, props }; },
    closeModal(type) { if (this.activeModal?.type === type) this.activeModal = null; },
  };
}

afterEach(() => vi.useRealTimers());

describe("presentationController", () => {
  it("owns toast expiry without creating a second reactive value", () => {
    vi.useFakeTimers();
    const ui = createUi();
    const controller = createPresentationController({ ui, toastDurationMs: 50 });

    controller.showToast("saved");
    expect(ui.toast).toBe("saved");
    vi.advanceTimersByTime(50);
    expect(ui.toast).toBe("");
  });

  it("only mirrors password attempt IDs supplied by document lifecycle", () => {
    const ui = createUi();
    const controller = createPresentationController({ ui });

    controller.showPasswordAttempt({ attemptId: 7, candidateName: "candidate.pdf", error: "" });
    expect(ui.activeModal?.props).toEqual({ attemptId: 7, candidateName: "candidate.pdf", error: "" });
    controller.dismissPasswordAttempt({ attemptId: 6 });
    expect(ui.activeModal?.props.attemptId).toBe(7);
    controller.showPasswordError({ attemptId: 6, candidateName: "old.pdf", error: "stale" });
    expect(ui.activeModal?.props.attemptId).toBe(7);
    controller.dismissPasswordAttempt({ attemptId: 7 });
    expect(ui.activeModal).toBeNull();
  });
});
