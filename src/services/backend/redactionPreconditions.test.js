import { describe, it, expect, vi } from "vitest";
import { readFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";
import { featureRegistry } from "../../featureRegistry.js";
import { guardFeatureAction, FeatureUnavailableError } from "../../featureActionGuard.js";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockRejectedValue({
    code: "FEATURE_UNAVAILABLE",
    message: "Güvenli redaksiyon bu sürümde devre dışıdır",
  }),
}));

describe("Prompt 8B-1 — Redaction Preconditions & Capability Invariants", () => {
  const root = process.cwd();

  it("keeps secure redaction experimental (never stable) and the visual mask a disabled mock", () => {
    const feature = featureRegistry.redaction;
    expect(feature).toBeDefined();
    expect(feature.enabled).toBe(true);
    expect(feature.status).toBe("experimental");

    const visualMask = featureRegistry.visual_redaction_mask;
    expect(visualMask.status).toBe("mock");
    expect(visualMask.enabled).toBe(false);
  });

  it("verifies 8A precondition: sensitive_data_scan is separate from redaction and is enabled", () => {
    const scanFeature = featureRegistry.sensitive_data_scan;
    expect(scanFeature).toBeDefined();
    expect(scanFeature.enabled).toBe(true);
    expect(scanFeature.status).not.toBe("disabled");
  });

  it("legacy apply_redactions action stays FEATURE_UNAVAILABLE now that secure redaction is enabled", async () => {
    const { applyRedactionsAction } = await import("../actions.js");
    await expect(
      applyRedactionsAction("sid", 1, "/out.pdf", [], { build: "production", platform: "macos" })
    ).rejects.toSatisfy((err) => err instanceof FeatureUnavailableError && err.code === "FEATURE_UNAVAILABLE");
    const { invoke } = await import("@tauri-apps/api/core");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("visual_redaction_mask guard rejects with FEATURE_UNAVAILABLE", async () => {
    const callback = vi.fn();
    await expect(
      guardFeatureAction("visual_redaction_mask", { platform: "macos" }, callback)
    ).rejects.toSatisfy((err) => {
      return (
        err instanceof FeatureUnavailableError &&
        err.code === "FEATURE_UNAVAILABLE"
      );
    });
    expect(callback).not.toHaveBeenCalled();
  });

  it("verifies 8A precondition: production UI files contain no apply_redactions callsite or visual mask overlay", () => {
    const pdfPageSource = readFileSync(
      resolve(root, "src/components/PdfPage.svelte"),
      "utf-8"
    );
    expect(pdfPageSource).not.toContain("redactionMaskPresentation");
    expect(pdfPageSource).not.toContain("apply_redactions");
    expect(pdfPageSource).not.toContain("applyRedactions");

    const appSource = readFileSync(resolve(root, "src/App.svelte"), "utf-8");
    expect(appSource).not.toContain("applyRedactions");
    expect(appSource).not.toContain("apply_redactions");
  });

  it("verifies docs/engineering/secure-redaction.md exists and contains capability matrix & initial safe profile", () => {
    const docPath = resolve(root, "docs/engineering/secure-redaction.md");
    expect(existsSync(docPath)).toBe(true);

    const docContent = readFileSync(docPath, "utf-8");
    expect(docContent).toContain("Capability Matrisi");
    expect(docContent).toContain("secure_rasterized_pages_v1");
    expect(docContent).toContain("pdfium-render");
    expect(docContent).toContain("lopdf");
    expect(docContent).toContain("FPDF_ANNOT_REDACT");
  });

  it("verifies redaction fixture manifest completeness", () => {
    const manifestPath = resolve(
      root,
      "src-tauri/tests/fixtures/redaction/manifest.json"
    );
    expect(existsSync(manifestPath)).toBe(true);

    const manifest = JSON.parse(readFileSync(manifestPath, "utf-8"));
    expect(manifest.version).toBe(1);
    expect(manifest.fixtures.length).toBe(26);

    const requiredFixtureIds = [
      "simple-text",
      "rotations",
      "same-size-same-rotation-pages",
      "mixed-secure-redaction-pages",
      "cropbox-mediabox-diff",
      "explicit-print-boxes",
      "nonzero-origin",
      "negative-origin",
      "image-only",
      "text-image",
      "annotations-fixture",
      "acroform-widget",
      "xfa-fixture",
      "encrypted-redaction",
      "signature-marked-byterange",
      "incremental-revision",
      "object-stream",
      "inherited-geometry",
      "inherited-resources",
      "embedded-files",
      "non-js-openaction",
      "js-action",
      "stream-literal-prev",
      "unresolved-acroform",
      "unresolved-resources",
      "whole-object-adversarial",
    ];

    const fixtureMap = new Map(manifest.fixtures.map((f) => [f.id, f]));

    for (const id of requiredFixtureIds) {
      const fixture = fixtureMap.get(id);
      expect(fixture, `Missing fixture entry for ${id}`).toBeDefined();
      expect(fixture.purpose).toBeTruthy();
      expect(fixture.expected_planning_outcome).toBeTruthy();
      expect(fixture.expected_profile).toBe("secure_rasterized_pages_v1");
      expect(fixture.expected_strategy).toBe("rasterized_pages");
      expect(fixture.expected_verification_outcome).toBeTruthy();
      expect(fixture.semantic_assertions).toBeInstanceOf(Array);
      expect(fixture.semantic_assertions.length).toBeGreaterThan(0);

      const fixtureFile = resolve(
        root,
        "src-tauri/tests/fixtures/redaction",
        fixture.filename
      );
      expect(
        existsSync(fixtureFile),
        `Missing fixture file ${fixture.filename}`
      ).toBe(true);
    }
  });
});
