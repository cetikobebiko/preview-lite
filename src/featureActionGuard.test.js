import { describe, expect, it, vi } from "vitest";
import {
  FeatureUnavailableError,
  guardFeatureAction,
} from "./featureActionGuard.js";

const developmentMac = { build: "development", platform: "macos" };

describe("feature action guard", () => {
  it("rejects with a typed error before IPC or success state can run", async () => {
    const invoke = vi.fn();
    const showSuccessToast = vi.fn();
    const closeModal = vi.fn();
    const state = { dirty: true, completed: false };

    await expect(
      guardFeatureAction("pades_signing", developmentMac, async () => {
        await invoke("sign_pdf_pades");
        showSuccessToast("İmzalandı");
        closeModal();
        state.dirty = false;
        state.completed = true;
      }),
    ).rejects.toBeInstanceOf(FeatureUnavailableError);

    expect(invoke).not.toHaveBeenCalled();
    expect(showSuccessToast).not.toHaveBeenCalled();
    expect(closeModal).not.toHaveBeenCalled();
    expect(state).toEqual({ dirty: true, completed: false });
  });

  it("runs an enabled feature and preserves its return value", async () => {
    const callback = vi.fn().mockResolvedValue("done");
    await expect(
      guardFeatureAction("search", developmentMac, callback),
    ).resolves.toBe("done");
    expect(callback).toHaveBeenCalledOnce();
  });

  it("fails closed for an unknown feature id", async () => {
    const callback = vi.fn();
    await expect(
      guardFeatureAction("missing_feature", developmentMac, callback),
    ).rejects.toMatchObject({
      name: "FeatureUnavailableError",
      code: "FEATURE_UNAVAILABLE",
      featureId: "missing_feature",
    });
    expect(callback).not.toHaveBeenCalled();
  });
});
