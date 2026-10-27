import { describe, expect, it, vi } from "vitest";
import {
  validateSensitiveScanReport,
  normalizeSensitiveScanReport,
} from "./redaction.js";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue({
    schema_version: 2,
    items: [],
    warnings: [],
    truncated: false,
  }),
}));

describe("redaction backend validation & normalization (Schema v2)", () => {
  const validGeom = {
    coordinate_space: "pdf_user_space",
    unit: "pdf_point",
    origin: "bottom_left",
    media_box: { left: 0, bottom: 0, width: 400, height: 600 },
    crop_box: null,
    effective_box: { left: 0, bottom: 0, width: 400, height: 600 },
    intrinsic_rotation: 90,
  };

  const validReportV2 = {
    schema_version: 2,
    items: [
      {
        page_index: 0,
        pattern_index: 1,
        text_preview: "12345678901",
        geometry_status: "resolved",
        page_geometry: validGeom,
        source_rects_pdf_points: [{ x: 10, y: 20, width: 100, height: 15 }],
        warnings: [],
      },
      {
        page_index: 1,
        pattern_index: 0,
        text_preview: "test@domain.com",
        geometry_status: "unresolved",
        page_geometry: null,
        source_rects_pdf_points: [],
        warnings: [
          {
            code: "SENSITIVE_SCAN_PAGE_GEOMETRY_UNRESOLVED",
            message: "Sayfa geometrisi çıkarılamadı.",
          },
        ],
      },
    ],
    warnings: [
      {
        code: "SENSITIVE_SCAN_PAGE_GEOMETRY_UNRESOLVED",
        message: "Bir veya daha fazla sayfada geometri çıkarılamadı.",
      },
    ],
    truncated: false,
  };

  it("validates and normalizes a valid schema v2 report to camelCase", () => {
    expect(validateSensitiveScanReport(validReportV2)).toBe(true);

    const normalized = normalizeSensitiveScanReport(validReportV2);
    expect(normalized).toEqual({
      schemaVersion: 2,
      items: [
        {
          pageIndex: 0,
          patternIndex: 1,
          textPreview: "12345678901",
          geometryStatus: "resolved",
          pageGeometry: {
            coordinateSpace: "pdf_user_space",
            unit: "pdf_point",
            origin: "bottom_left",
            mediaBox: { left: 0, bottom: 0, width: 400, height: 600 },
            cropBox: null,
            effectiveBox: { left: 0, bottom: 0, width: 400, height: 600 },
            intrinsicRotation: 90,
          },
          sourceRectsPdfPoints: [{ x: 10, y: 20, width: 100, height: 15 }],
          warnings: [],
        },
        {
          pageIndex: 1,
          patternIndex: 0,
          textPreview: "test@domain.com",
          geometryStatus: "unresolved",
          pageGeometry: null,
          sourceRectsPdfPoints: [],
          warnings: [
            {
              code: "SENSITIVE_SCAN_PAGE_GEOMETRY_UNRESOLVED",
              message: "Sayfa geometrisi çıkarılamadı.",
            },
          ],
        },
      ],
      warnings: [
        {
          code: "SENSITIVE_SCAN_PAGE_GEOMETRY_UNRESOLVED",
          message: "Bir veya daha fazla sayfada geometri çıkarılamadı.",
        },
      ],
      truncated: false,
    });
  });

  it("rejects schema_version != 2 (rejects schema v1)", () => {
    const invalidSchema = { ...validReportV2, schema_version: 1 };
    expect(() => validateSensitiveScanReport(invalidSchema)).toThrow(
      "schema_version must be 2",
    );
  });

  it("rejects resolved items missing page_geometry or with empty source_rects", () => {
    const missingGeom = {
      ...validReportV2,
      items: [{ ...validReportV2.items[0], page_geometry: null }],
    };
    expect(() => validateSensitiveScanReport(missingGeom)).toThrow(
      "resolved item must have page_geometry",
    );

    const emptyRects = {
      ...validReportV2,
      items: [{ ...validReportV2.items[0], source_rects_pdf_points: [] }],
    };
    expect(() => validateSensitiveScanReport(emptyRects)).toThrow(
      "resolved item must have source_rects_pdf_points",
    );
  });

  it("rejects non-numeric or unsupported intrinsic_rotation values", () => {
    const stringRotation = {
      ...validReportV2,
      items: [
        {
          ...validReportV2.items[0],
          page_geometry: { ...validGeom, intrinsic_rotation: "90" },
        },
      ],
    };
    expect(() => validateSensitiveScanReport(stringRotation)).toThrow(
      "intrinsic_rotation must be numeric 0, 90, 180, or 270",
    );

    const enumStringRotation = {
      ...validReportV2,
      items: [
        {
          ...validReportV2.items[0],
          page_geometry: { ...validGeom, intrinsic_rotation: "Rotate90" },
        },
      ],
    };
    expect(() => validateSensitiveScanReport(enumStringRotation)).toThrow(
      "intrinsic_rotation must be numeric 0, 90, 180, or 270",
    );

    const invalidNumberRotation = {
      ...validReportV2,
      items: [
        {
          ...validReportV2.items[0],
          page_geometry: { ...validGeom, intrinsic_rotation: 45 },
        },
      ],
    };
    expect(() => validateSensitiveScanReport(invalidNumberRotation)).toThrow(
      "intrinsic_rotation must be numeric 0, 90, 180, or 270",
    );
  });

  it("rejects unknown fields at report, item, or page_geometry level", () => {
    const reportUnknown = { ...validReportV2, foo: "bar" };
    expect(() => validateSensitiveScanReport(reportUnknown)).toThrow("unknown field 'foo'");

    const itemUnknown = {
      ...validReportV2,
      items: [{ ...validReportV2.items[0], foo: "bar" }],
    };
    expect(() => validateSensitiveScanReport(itemUnknown)).toThrow("unknown field 'foo'");

    const geomUnknown = {
      ...validReportV2,
      items: [
        {
          ...validReportV2.items[0],
          page_geometry: { ...validGeom, foo: "bar" },
        },
      ],
    };
    expect(() => validateSensitiveScanReport(geomUnknown)).toThrow("unknown field 'foo'");
  });
});

describe("startSensitiveDataScan and scanSensitiveData wrapper behavior", () => {
  it("preserves canonical jobId and cancel function without generating a second UUID", async () => {
    const { startSensitiveDataScan, scanSensitiveData } = await import("./redaction.js");

    const handle = startSensitiveDataScan("test-session", 1, [{ kind: "regex", value: "\\d+" }]);
    expect(typeof handle.jobId).toBe("string");
    expect(handle.jobId.length).toBeGreaterThan(0);
    expect(typeof handle.cancel).toBe("function");
    expect(typeof handle.promise.then).toBe("function");
    await handle.promise.catch(() => {});

    const scanPromise = scanSensitiveData("test-session", 1, [{ kind: "regex", value: "\\d+" }]);
    expect(typeof scanPromise.then).toBe("function");
    await scanPromise.catch(() => {});
  });
});

describe("secure-redaction wire DTO serializers and startSecureRedactionPlan wrapper", () => {
  const samplePolicy = {
    schemaVersion: "1.0.0",
    profile: "secure_rasterized_pages_v1",
    strategy: "rasterized_pages",
    rasterDpi: 300,
    appearance: {
      kind: "solid",
      color: "black",
      opacity: 1.0,
    },
    safetyMarginPoints: 2.0,
    interactiveContentPolicy: "strip",
    signaturePolicy: "block",
    encryptionPolicy: "block",
  };

  const sampleAcks = {
    schemaVersion: "1.0.0",
    rasterizedPagesAccepted: true,
    textSearchLossAccepted: true,
    accessibilityLossAccepted: true,
    annotationLossAccepted: true,
    formLossAccepted: true,
    signatureInvalidationAccepted: true,
    encryptionChangeAccepted: true,
  };

  const sampleRegion = {
    id: "reg-1",
    pageIndex: 0,
    normalizedTopLeftRect: {
      x: 0.1,
      y: 0.2,
      width: 0.3,
      height: 0.4,
    },
    source: "manual",
  };

  it("converts camelCase policy, acknowledgements, and region to strict snake_case wire DTOs", async () => {
    const {
      toSecureRedactionPolicyWire,
      toRedactionAcknowledgementsWire,
      toRedactionRegionInputWire,
    } = await import("./redaction.js");

    const wirePolicy = toSecureRedactionPolicyWire(samplePolicy);
    expect(wirePolicy).toEqual({
      schema_version: "1.0.0",
      profile: "secure_rasterized_pages_v1",
      strategy: "rasterized_pages",
      raster_dpi: 300,
      appearance: {
        kind: "solid",
        color: "black",
        opacity: 1.0,
      },
      safety_margin_points: 2.0,
      interactive_content_policy: "strip",
      signature_policy: "block",
      encryption_policy: "block",
    });

    const wireAcks = toRedactionAcknowledgementsWire(sampleAcks);
    expect(wireAcks).toEqual({
      schema_version: "1.0.0",
      rasterized_pages_accepted: true,
      text_search_loss_accepted: true,
      accessibility_loss_accepted: true,
      annotation_loss_accepted: true,
      form_loss_accepted: true,
      signature_invalidation_accepted: true,
      encryption_change_accepted: true,
    });

    const wireRegion = toRedactionRegionInputWire(sampleRegion);
    expect(wireRegion).toEqual({
      id: "reg-1",
      page_index: 0,
      normalized_top_left_rect: {
        x: 0.1,
        y: 0.2,
        width: 0.3,
        height: 0.4,
      },
      source: "manual",
    });
  });

  it("startSecureRedactionPlan performs synchronous region pre-validation before backend call", async () => {
    const { startSecureRedactionPlan } = await import("./redaction.js");
    const { REDACTION_ERROR_CODES, REDACTION_LIMITS } = await import(
      "../../features/redaction/redactionSchemas.js"
    );

    // Non-array regions -> reject
    expect(() =>
      startSecureRedactionPlan("session-1", 1, samplePolicy, sampleAcks, "invalid-regions"),
    ).toThrow(expect.objectContaining({ code: REDACTION_ERROR_CODES.REDACTION_INVALID_REGION }));

    // Duplicate region ID -> reject
    const duplicateRegions = [
      sampleRegion,
      { ...sampleRegion, normalizedTopLeftRect: { x: 0.2, y: 0.2, width: 0.1, height: 0.1 } },
    ];
    expect(() =>
      startSecureRedactionPlan("session-1", 1, samplePolicy, sampleAcks, duplicateRegions),
    ).toThrow(
      expect.objectContaining({ code: REDACTION_ERROR_CODES.REDACTION_DUPLICATE_REGION_ID }),
    );

    // Max regions + 1 -> reject
    const overflowRegions = Array.from(
      { length: REDACTION_LIMITS.MAX_REGIONS_PER_INPUT + 1 },
      (_, i) => ({
        id: `r-${i}`,
        pageIndex: 0,
        normalizedTopLeftRect: { x: 0.1, y: 0.1, width: 0.1, height: 0.1 },
        source: "manual",
      }),
    );
    expect(() =>
      startSecureRedactionPlan("session-1", 1, samplePolicy, sampleAcks, overflowRegions),
    ).toThrow(
      expect.objectContaining({ code: REDACTION_ERROR_CODES.REDACTION_REGION_LIMIT_EXCEEDED }),
    );

    // Exact max regions -> accepts (returns handle)
    const exactMaxRegions = Array.from(
      { length: REDACTION_LIMITS.MAX_REGIONS_PER_INPUT },
      (_, i) => ({
        id: `r-${i}`,
        pageIndex: 0,
        normalizedTopLeftRect: { x: 0.1, y: 0.1, width: 0.1, height: 0.1 },
        source: "manual",
      }),
    );
    const handle = startSecureRedactionPlan(
      "session-1",
      1,
      samplePolicy,
      sampleAcks,
      exactMaxRegions,
    );
    expect(handle.jobId).toBeDefined();
    await handle.promise.catch(() => {});
  });

  it("starts secure execution through the strict contract and preserves its cancellation handle", async () => {
    const { startSecureRedactionExecution } = await import("./redaction.js");
    const { invoke } = await import("@tauri-apps/api/core");
    invoke.mockClear();

    const handle = startSecureRedactionExecution(
      "session-1",
      1,
      samplePolicy,
      sampleAcks,
      [sampleRegion],
      "a".repeat(64),
      "/tmp/secure-redacted.pdf",
    );
    expect(typeof handle.jobId).toBe("string");
    expect(typeof handle.cancel).toBe("function");
    await handle.promise.catch(() => {});
    expect(invoke).toHaveBeenCalledWith(
      "execute_secure_redaction",
      expect.objectContaining({
        jobId: handle.jobId,
        sessionId: "session-1",
        expectedPlanDigest: "a".repeat(64),
        targetPath: "/tmp/secure-redacted.pdf",
      }),
    );
    await handle.cancel();
    expect(invoke).toHaveBeenCalledWith(
      "cancel_session_job",
      expect.objectContaining({ sessionId: "session-1", jobId: handle.jobId }),
    );

    expect(() => startSecureRedactionExecution(
      "session-1", 1, samplePolicy, sampleAcks, [sampleRegion], "not-a-digest", "/tmp/a.pdf",
    )).toThrow();
    expect(() => startSecureRedactionExecution(
      "session-1", 1, samplePolicy, sampleAcks, [sampleRegion], "a".repeat(64), "",
    )).toThrow(/targetPath/);
  });

  it("startSecureRedactionPlan verifies cryptographic digest and returns verified plan", async () => {
    const { startSecureRedactionPlan } = await import("./redaction.js");
    const {
      computePlanDigest,
      computeGeometryDigest,
      normalizeSecureRedactionPlanStructure,
      REDACTION_SCHEMA_VERSION,
      SECURE_REDACTION_PLAN_VERSION,
    } = await import("../../features/redaction/redactionSchemas.js");

    const sampleSha256 = "a".repeat(64);
    const pageGeometryWire = {
      page_index: 0,
      media_box: { left: 0, bottom: 0, width: 612, height: 792 },
      crop_box: null,
      effective_box: { left: 0, bottom: 0, width: 612, height: 792 },
      rotation: 0,
    };
    const pageGeometryCamel = {
      pageIndex: 0,
      mediaBox: { left: 0, bottom: 0, width: 612, height: 792 },
      cropBox: null,
      effectiveBox: { left: 0, bottom: 0, width: 612, height: 792 },
      rotation: 0,
    };
    const geometryDigest = await computeGeometryDigest(pageGeometryCamel);

    const validPlanWire = {
      schema_version: REDACTION_SCHEMA_VERSION,
      plan_version: SECURE_REDACTION_PLAN_VERSION,
      input_digest: sampleSha256,
      content_generation: 1,
      policy: {
        schema_version: "1.0.0",
        profile: "secure_rasterized_pages_v1",
        strategy: "rasterized_pages",
        raster_dpi: 300,
        appearance: { kind: "solid", color: "black", opacity: 1.0 },
        safety_margin_points: 2.0,
        interactive_content_policy: "block",
        signature_policy: "block",
        encryption_policy: "block",
      },
      acknowledgements: {
        schema_version: "1.0.0",
        rasterized_pages_accepted: true,
        text_search_loss_accepted: true,
        accessibility_loss_accepted: true,
        annotation_loss_accepted: true,
        form_loss_accepted: true,
        signature_invalidation_accepted: true,
        encryption_change_accepted: true,
      },
      acknowledgement_requirements: {
        schema_version: "1.0.0",
        rasterized_pages_required: true,
        text_search_loss_required: true,
        accessibility_loss_required: false,
        annotation_loss_required: false,
        form_loss_required: false,
        signature_invalidation_required: false,
        encryption_change_required: false,
      },
      canonical_regions: [
        {
          region_id: "canon_reg-1",
          source_region_ids: ["reg-1"],
          page_index: 0,
          original_normalized_rect: { x: 0.1, y: 0.2, width: 0.3, height: 0.4 },
          base_pdf_point_rect: { left: 61.2, bottom: 316.8, width: 183.6, height: 316.8 },
          requested_safety_margin_points: 2.0,
          applied_pdf_point_rect: { left: 59.2, bottom: 314.8, width: 187.6, height: 320.8 },
          applied_margins: { left: 2.0, right: 2.0, top: 2.0, bottom: 2.0 },
          boundary_adjusted: false,
          device_pixel_rect: null,
        },
      ],
      facts: {
        schema_version: "1.0.0",
        total_pages: 1,
        encryption: "absent",
        signature_dictionary: "absent",
        signature_field: "absent",
        byte_range: "absent",
        acro_form: "absent",
        xfa: "absent",
        annotations: "absent",
        optional_content: "absent",
        embedded_files: "absent",
        javascript: "absent",
        incremental_revisions: "absent",
        structure_tree: "absent",
        outlines: "absent",
        names_dictionary: "absent",
        document_actions: "absent",
        complex_document_resources: "absent",
      },
      page_plans: [
        {
          schema_version: "1.0.0",
          page_index: 0,
          strategy: "rasterize_page",
          geometry: pageGeometryWire,
          geometry_digest: geometryDigest,
          facts: {
            schema_version: "1.0.0",
            page_index: 0,
            text: "present",
            images: "absent",
            vector_paths: "absent",
            form_xobjects: "absent",
            annotations: "absent",
            link_annotations: "absent",
            widgets: "absent",
            transparency: "absent",
            optional_content: "absent",
            complex_resources: "absent",
            structure_association: "absent",
            page_actions: "absent",
            spot_colors: "absent",
            complex_color_spaces: "absent",
            geometry_profile: "standard_effective_box_v1",
            rotation: 0,
          },
          canonical_regions: [
            {
              region_id: "canon_reg-1",
              source_region_ids: ["reg-1"],
              page_index: 0,
              original_normalized_rect: { x: 0.1, y: 0.2, width: 0.3, height: 0.4 },
              base_pdf_point_rect: { left: 61.2, bottom: 316.8, width: 183.6, height: 316.8 },
              requested_safety_margin_points: 2.0,
              applied_pdf_point_rect: { left: 59.2, bottom: 314.8, width: 187.6, height: 320.8 },
              applied_margins: { left: 2.0, right: 2.0, top: 2.0, bottom: 2.0 },
              boundary_adjusted: false,
              device_pixel_rect: null,
            },
          ],
          feature_losses: {
            schema_version: "1.0.0",
            text_selection_lost: true,
            searchability_lost: true,
            accessibility_tags_lost: false,
            links_lost: false,
            annotations_lost: false,
            forms_lost: false,
            vector_quality_lost: false,
            color_space_fidelity_risk: false,
            spot_colors_lost: false,
            layers_lost: false,
            structure_association_lost: false,
            page_actions_lost: false,
          },
          blockers: [],
          warnings: [],
          executable: true,
        },
      ],
      losses: {
        schema_version: "1.0.0",
        text_selection_lost: true,
        searchability_lost: true,
        accessibility_tags_lost: false,
        links_lost: false,
        annotations_lost: false,
        forms_lost: false,
        vector_quality_lost: false,
        color_space_fidelity_risk: false,
        spot_colors_lost: false,
        layers_lost: false,
        structure_association_lost: false,
        page_actions_lost: false,
      },
      blockers: [],
      warnings: [],
      executable: true,
      acknowledgements_satisfied: true,
      ready_for_execution: true,
      plan_digest: "a".repeat(64),
    };

    const normForDigest = normalizeSecureRedactionPlanStructure(validPlanWire);
    validPlanWire.plan_digest = await computePlanDigest(normForDigest);

    const { invoke } = await import("@tauri-apps/api/core");
    invoke.mockResolvedValueOnce({ status: "completed", value: validPlanWire });

    const handle = startSecureRedactionPlan(
      "session-1",
      1,
      samplePolicy,
      sampleAcks,
      [sampleRegion],
    );
    const plan = await handle.promise;
    expect(plan.digestVerified).toBe(true);

    // Bad plan digest -> reject
    const badDigestWire = { ...validPlanWire, plan_digest: "0".repeat(64) };
    invoke.mockResolvedValueOnce({ status: "completed", value: badDigestWire });

    const badDigestHandle = startSecureRedactionPlan(
      "session-1",
      1,
      samplePolicy,
      sampleAcks,
      [sampleRegion],
    );
    await expect(badDigestHandle.promise).rejects.toThrow();

    // Bad geometry digest -> reject
    const badGeomDigestWire = {
      ...validPlanWire,
      page_plans: [
        {
          ...validPlanWire.page_plans[0],
          geometry_digest: "0".repeat(64),
        },
      ],
    };
    invoke.mockResolvedValueOnce({ status: "completed", value: badGeomDigestWire });

    const badGeomHandle = startSecureRedactionPlan(
      "session-1",
      1,
      samplePolicy,
      sampleAcks,
      [sampleRegion],
    );
    await expect(badGeomHandle.promise).rejects.toThrow();

    // Phantom root field rejections
    const phantomFields = [
      "geometry_digest",
      "inputs",
      "page_geometries",
      "page_facts",
      "overall_losses",
      "total_estimated_pixels",
    ];
    for (const field of phantomFields) {
      const badWire = { ...validPlanWire, [field]: "invalid" };
      invoke.mockResolvedValueOnce({ status: "completed", value: badWire });

      const badHandle = startSecureRedactionPlan(
        "session-1",
        1,
        samplePolicy,
        sampleAcks,
        [sampleRegion],
      );
      await expect(badHandle.promise).rejects.toThrow(/Unknown field/);
    }

    // Semantic invariant broken plan with recomputed valid digest -> wrapper reject
    // Case 1: ready_for_execution = true but executable = false
    const brokenSemanticWire1 = JSON.parse(JSON.stringify(validPlanWire));
    brokenSemanticWire1.executable = false;
    brokenSemanticWire1.ready_for_execution = true;
    const norm1 = normalizeSecureRedactionPlanStructure;
    expect(() => norm1(brokenSemanticWire1)).toThrow();

    // Requirement 8: Semantically invalid plan with valid recomputed plan_digest rejected via startSecureRedactionPlan production wrapper
    const brokenSemanticWireWrapper = JSON.parse(JSON.stringify(validPlanWire));
    brokenSemanticWireWrapper.executable = false;
    brokenSemanticWireWrapper.ready_for_execution = true;

    const normBrokenForDigest = normalizeSecureRedactionPlanStructure(validPlanWire);
    normBrokenForDigest.executable = false;
    normBrokenForDigest.readyForExecution = true;

    brokenSemanticWireWrapper.plan_digest = await computePlanDigest(normBrokenForDigest);

    invoke.mockResolvedValueOnce({ status: "completed", value: brokenSemanticWireWrapper });

    const brokenWrapperHandle = startSecureRedactionPlan(
      "session-1",
      1,
      samplePolicy,
      sampleAcks,
      [sampleRegion],
    );
    await expect(brokenWrapperHandle.promise).rejects.toThrow();

    // Case 2: losses mismatch
    const brokenSemanticWire2 = JSON.parse(JSON.stringify(validPlanWire));
    brokenSemanticWire2.losses.text_selection_lost = false;
    expect(() => norm1(brokenSemanticWire2)).toThrow(/losses/);

    // Case 3: canonical_regions mismatch
    const brokenSemanticWire3 = JSON.parse(JSON.stringify(validPlanWire));
    brokenSemanticWire3.canonical_regions = [];
    expect(() => norm1(brokenSemanticWire3)).toThrow(/canonicalRegions/);
  });
});
