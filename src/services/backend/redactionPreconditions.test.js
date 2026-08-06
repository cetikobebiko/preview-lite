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

  it("verifies 8A precondition: redaction feature is disabled", () => {
    const feature = featureRegistry.redaction;
    expect(feature).toBeDefined();
    expect(feature.enabled).toBe(false);
    expect(feature.status).toBe("disabled");
  });

  it("verifies 8A precondition: sensitive_data_scan is separate from redaction and is enabled", () => {
    const scanFeature = featureRegistry.sensitive_data_scan;
    expect(scanFeature).toBeDefined();
    expect(scanFeature.enabled).toBe(true);
    expect(scanFeature.status).not.toBe("disabled");
  });

  it("verifies 8A precondition: guardFeatureAction for redaction rejects with FEATURE_UNAVAILABLE", async () => {
    const callback = vi.fn();
    await expect(
      guardFeatureAction("redaction", { platform: "macos" }, callback)
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
    expect(manifest.fixtures.length).toBe(14);

    const requiredFixtureIds = [
      "simple-text",
      "rotations",
      "cropbox-mediabox-diff",
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
