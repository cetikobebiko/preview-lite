import { describe, expect, it } from "vitest";
import {
  getFeaturePresentation,
  isFeatureRunnable,
  isFeatureVisible,
} from "./featureRegistry.js";

describe("platform-limited feature visibility", () => {
  it("shows and runs a macOS feature on macOS", () => {
    const context = { build: "production", platform: "macos" };
    expect(isFeatureVisible("macos_share", context)).toBe(true);
    expect(isFeatureRunnable("macos_share", context)).toBe(true);
  });

  it.each(["windows", "linux"])(
    "hides a macOS feature on %s",
    (platform) => {
      const context = { build: "development", platform };
      expect(isFeatureVisible("macos_share", context)).toBe(false);
      expect(isFeatureRunnable("macos_share", context)).toBe(false);
    },
  );

  it("hides a platform-limited feature on an unknown platform", () => {
    const context = { build: "development", platform: "unknown" };
    expect(isFeatureVisible("macos_share", context)).toBe(false);
    expect(isFeatureRunnable("macos_share", context)).toBe(false);
  });

  it("shows a platform-compatible mock disabled in development", () => {
    const presentation = getFeaturePresentation("visual_redaction_mask", {
      build: "development",
      platform: "macos",
    });
    expect(presentation.visible).toBe(true);
    expect(presentation.runnable).toBe(false);
    expect(presentation.badge).toBe("MOCK");
  });
});
