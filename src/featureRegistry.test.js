import { describe, expect, it } from "vitest";
import {
  featureRegistry,
  getFeature,
  getFeaturePresentation,
  isFeatureRecordRunnable,
  isFeatureRunnable,
  isFeatureVisible,
  validateFeatureRegistryRecords,
} from "./featureRegistry.js";

const productionMac = { build: "production", platform: "macos" };
const developmentMac = { build: "development", platform: "macos" };

describe("feature registry metadata", () => {
  it("contains complete, immutable records with unique ids", () => {
    expect(Object.isFrozen(featureRegistry)).toBe(true);
    const ids = Object.keys(featureRegistry);
    expect(ids.length).toBeGreaterThanOrEqual(20);
    expect(new Set(ids).size).toBe(ids.length);

    for (const feature of Object.values(featureRegistry)) {
      expect(Object.keys(feature).sort()).toEqual([
        "documentation",
        "enabled",
        "id",
        "label",
        "platform",
        "status",
        "warning",
      ]);
      expect(Object.isFrozen(feature)).toBe(true);
      expect(Object.isFrozen(feature.platform)).toBe(true);
    }
  });

  it("rejects missing, duplicate, and malformed metadata", () => {
    const valid = {
      id: "valid_feature",
      label: "Valid",
      status: "experimental",
      enabled: true,
      warning: "Warning",
      platform: ["macos"],
      documentation: "docs/example.md",
    };

    expect(() => validateFeatureRegistryRecords([{ ...valid, warning: undefined }])).toThrow();
    expect(() => validateFeatureRegistryRecords([valid, { ...valid }])).toThrow(/Duplicate/);
    expect(() => validateFeatureRegistryRecords([{ ...valid, status: "working" }])).toThrow();
    expect(() => validateFeatureRegistryRecords([{ ...valid, enabled: "yes" }])).toThrow();
    const { documentation: _removed, ...missingDocumentation } = valid;
    expect(() => validateFeatureRegistryRecords([missingDocumentation])).toThrow(/documentation/);
  });
});

describe("feature policy", () => {
  it("never presents mock features as stable or runnable", () => {
    const feature = getFeature("pades_signing");
    expect(feature.status).toBe("mock");
    expect(isFeatureRunnable(feature.id, developmentMac)).toBe(false);
    expect(getFeaturePresentation(feature.id, developmentMac).badge).toBe("MOCK");
    expect(isFeatureRecordRunnable({
      ...feature,
      enabled: true,
    }, developmentMac)).toBe(false);
    expect(isFeatureRecordRunnable({
      ...getFeature("form_editing"),
      enabled: true,
    }, developmentMac)).toBe(false);
  });

  it("hides mock features in production and shows them disabled in development", () => {
    expect(isFeatureVisible("pades_signing", productionMac)).toBe(false);
    const presentation = getFeaturePresentation("pades_signing", developmentMac);
    expect(presentation.visible).toBe(true);
    expect(presentation.runnable).toBe(false);
    expect(presentation.warning).toContain("gerçek işlem yapılmaz");
  });

  it("does not make disabled features runnable", () => {
    const presentation = getFeaturePresentation("form_editing", developmentMac);
    expect(presentation.visible).toBe(true);
    expect(presentation.runnable).toBe(false);
  });

  it("hides annotation burn/export in production and only presents it disabled in development", () => {
    expect(isFeatureVisible("annotation_burn_export", productionMac)).toBe(false);
    expect(isFeatureRunnable("annotation_burn_export", productionMac)).toBe(false);

    expect(getFeaturePresentation("annotation_burn_export", developmentMac)).toMatchObject({
      visible: true,
      runnable: false,
    });
  });

  it("evaluates enabled separately from runtime platform for disabled features", () => {
    const continuityCamera = getFeature("macos_continuity_camera");
    expect(continuityCamera.enabled).toBe(false);
    expect(continuityCamera.status).toBe("disabled");
    expect(continuityCamera.platform).toEqual(["macos"]);
    expect(isFeatureRunnable(continuityCamera.id, productionMac)).toBe(false);
  });

  it("shows and runs a macOS feature only on macOS", () => {
    expect(isFeatureVisible("macos_share", productionMac)).toBe(true);
    expect(isFeatureRunnable("macos_share", productionMac)).toBe(true);

    for (const platform of ["windows", "linux", "unknown"]) {
      const context = { build: "development", platform };
      expect(isFeatureVisible("macos_share", context)).toBe(false);
      expect(isFeatureRunnable("macos_share", context)).toBe(false);
    }
  });

  it("still shows a platform-compatible mock disabled in development", () => {
    const presentation = getFeaturePresentation("pades_signing", developmentMac);
    expect(presentation.visible).toBe(true);
    expect(presentation.runnable).toBe(false);
    expect(presentation.badge).toBe("MOCK");
  });

  it("fails closed on unknown feature ids", () => {
    expect(isFeatureVisible("missing_feature", developmentMac)).toBe(false);
    expect(getFeaturePresentation("missing_feature", developmentMac).runnable).toBe(false);
  });

  it("allows a normal enabled experimental feature on a supported platform", () => {
    const presentation = getFeaturePresentation("pdf_viewer", productionMac);
    expect(presentation.visible).toBe(true);
    expect(presentation.runnable).toBe(true);
    expect(presentation.badge).toBe("EXPERIMENTAL");
    expect(presentation.warning).not.toBe("");
  });

  it("keeps Command Palette experimental and runnable", () => {
    const presentation = getFeaturePresentation("command_palette", productionMac);
    expect(presentation.status).toBe("experimental");
    expect(presentation.badge).toBe("EXPERIMENTAL");
    expect(presentation.visible).toBe(true);
    expect(presentation.runnable).toBe(true);
  });

  it("enforces exact metadata and runtime policy for redaction and sensitive data features", () => {
    const sensitiveScan = getFeature("sensitive_data_scan");
    expect(sensitiveScan.status).toBe("experimental");
    expect(sensitiveScan.enabled).toBe(true);
    expect(isFeatureRunnable("sensitive_data_scan", productionMac)).toBe(true);
    expect(isFeatureRunnable("sensitive_data_scan", developmentMac)).toBe(true);

    const redaction = getFeature("redaction");
    expect(redaction.status).toBe("disabled");
    expect(redaction.enabled).toBe(false);
    expect(isFeatureRunnable("redaction", productionMac)).toBe(false);
    expect(isFeatureRunnable("redaction", developmentMac)).toBe(false);

    const sanitization = getFeature("sanitization");
    expect(sanitization.status).toBe("disabled");
    expect(sanitization.enabled).toBe(false);
    expect(isFeatureRunnable("sanitization", productionMac)).toBe(false);
    expect(isFeatureRunnable("sanitization", developmentMac)).toBe(false);

    const visualMask = getFeature("visual_redaction_mask");
    expect(visualMask.status).toBe("mock");
    expect(visualMask.enabled).toBe(false);
    expect(isFeatureVisible("visual_redaction_mask", productionMac)).toBe(false);
    expect(isFeatureRunnable("visual_redaction_mask", developmentMac)).toBe(false);
  });
});
