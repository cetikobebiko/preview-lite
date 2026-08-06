import { describe, expect, it, vi } from "vitest";
import { createCommandActionRouter } from "./commandActionRouter.js";

describe("commandActionRouter", () => {
  it("routes known commands through their feature guard", async () => {
    const action = vi.fn();
    const guard = vi.fn(async (_id, _context, callback) => callback());
    const router = createCommandActionRouter({
      getCommand: () => ({ id: "export", featureId: "export_feature" }),
      actions: { export: action },
      guard,
      featureContext: { platform: "macos" },
      reportUnavailable: () => false,
      onUnknown: vi.fn(),
    });

    await expect(router.dispatch("export")).resolves.toEqual({ status: "handled" });
    expect(guard).toHaveBeenCalledWith("export_feature", { platform: "macos" }, action);
    expect(action).toHaveBeenCalledOnce();
  });

  it("does not execute unknown or unregistered commands", async () => {
    const onUnknown = vi.fn();
    const router = createCommandActionRouter({
      getCommand: () => ({ id: "missing" }),
      actions: {},
      guard: vi.fn(),
      featureContext: {},
      reportUnavailable: () => false,
      onUnknown,
    });

    await expect(router.dispatch("missing")).resolves.toEqual({ status: "unknown" });
    expect(onUnknown).toHaveBeenCalledWith("missing");
  });
});
