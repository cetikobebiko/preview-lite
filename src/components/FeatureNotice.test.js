import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { getFeaturePresentation } from "../featureRegistry.js";

describe("FeatureNotice presentation wiring", () => {
  const source = readFileSync(
    resolve(process.cwd(), "src/components/FeatureNotice.svelte"),
    "utf8",
  );

  it("wires visibility, badge, warning, and status to registry presentation", () => {
    expect(source).toContain("getFeaturePresentation(featureId, context)");
    expect(source).toContain("presentation.visible");
    expect(source).toContain("presentation.badge");
    expect(source).toContain("presentation.warning");
    expect(source).toContain("data-feature-status={presentation.status}");
  });

  it("does not render a production mock notice because the feature is hidden", () => {
    const presentation = getFeaturePresentation("visual_redaction_mask", {
      build: "production",
      platform: "macos",
    });

    expect(presentation.visible).toBe(false);
    expect(presentation.runnable).toBe(false);
  });

  it("presents a disabled developer mock with an explicit no-operation warning", () => {
    const presentation = getFeaturePresentation("visual_redaction_mask", {
      build: "development",
      platform: "macos",
    });

    expect(presentation.visible).toBe(true);
    expect(presentation.runnable).toBe(false);
    expect(presentation.badge).toBe("MOCK");
    expect(presentation.warning).toContain("gerçek işlem yapılmaz");
  });

  it("presents experimental status and warning copy", () => {
    const presentation = getFeaturePresentation("rgb_derived_print_preview", {
      build: "production",
      platform: "macos",
    });

    expect(presentation.badge).toBe("EXPERIMENTAL");
    expect(presentation.warning).toContain("Ekran RGB render'ından");
  });
});
