import { describe, it, expect } from 'vitest';
import * as fs from 'node:fs';
import * as path from 'node:path';
import {
  REDACTION_SCHEMA_VERSION,
  REDACTION_LIMITS,
  REDACTION_ERROR_CODES,
  U32_MIN,
  U32_MAX,
  I32_MIN,
  I32_MAX,
  RedactionError,
  normalizeNormalizedTopLeftRect,
  validateNormalizedTopLeftRect,
  normalizeRedactionRegionInput,
  validateRedactionRegionInput,
  validateRedactionRegionInputs,
  normalizeSecureRedactionPolicy,
  validateSecureRedactionPolicy,
  normalizeRedactionAcknowledgements,
  validateRedactionAcknowledgements,
  normalizeRedactionBlocker,
  normalizeRedactionWarning,
  normalizeRedactionVerificationCheck,
  normalizeSecureRedactionPublicationReport,
  validateUniqueBlockers,
  validateUniqueWarnings,
  validateUniqueVerificationChecks,
  normalizeDocumentRedactionFacts,
  normalizePageRedactionFacts,
  normalizeRedactionFeatureLosses,
  normalizeCanonicalPageGeometry,
  normalizeCanonicalRedactionRegion,
  normalizeDevicePixelRect,
  assertI32PlusU32DoesNotOverflow,
  normalizePageRedactionPlan,
  normalizeSecureRedactionPlan,
  validateSecureRedactionPlan,
  normalizeSecureRedactionTransformReport,
  validateSecureRedactionTransformReport,
  normalizeSecureRedactionVerificationReport,
  validateSecureRedactionVerificationReport,
  normalizeSecureRedactionExecutionReport,
  validateSecureRedactionExecutionReport,
  evaluateDocumentBlockers,
  getUtf8ByteLength,
  validateSha256Digest,
  assertU32,
  computeGeometryDigest,
  computePlanDigest,
} from './redactionSchemas.js';

describe('Secure Redaction Domain Contracts, Behavioral Parity & Adversarial Matrix (Prompt 8B-2.5)', () => {
  const sampleSha256 = 'a'.repeat(64);

  const sampleValidInput = (id = 'reg1', x = 0.1, y = 0.1, w = 0.5, h = 0.5) => ({
    id,
    pageIndex: 0,
    normalizedTopLeftRect: { x, y, width: w, height: h },
    source: 'manual_selection',
  });

  const sampleValidPolicy = () => ({
    schemaVersion: '1.0.0',
    profile: 'secure_rasterized_pages_v1',
    strategy: 'rasterized_pages',
    rasterDpi: 300,
    appearance: {
      kind: 'solid',
      color: 'black',
      opacity: 1.0,
    },
    safetyMarginPoints: 2.0,
    interactiveContentPolicy: 'block',
    signaturePolicy: 'block',
    encryptionPolicy: 'block',
  });

  const sampleValidAcknowledgements = () => ({
    schemaVersion: '1.0.0',
    rasterizedPagesAccepted: true,
    textSearchLossAccepted: true,
    accessibilityLossAccepted: true,
    annotationLossAccepted: true,
    formLossAccepted: true,
    signatureInvalidationAccepted: true,
    encryptionChangeAccepted: true,
  });

  const sampleValidDocumentFacts = () => ({
    schemaVersion: '1.0.0',
    totalPages: 1,
    encryption: 'absent',
    signatureDictionary: 'absent',
    signatureField: 'absent',
    byteRange: 'absent',
    acroForm: 'absent',
    xfa: 'absent',
    annotations: 'absent',
    optionalContent: 'absent',
    embeddedFiles: 'absent',
    javascript: 'absent',
    incrementalRevisions: 'absent',
    structureTree: 'absent',
    outlines: 'absent',
    namesDictionary: 'absent',
    documentActions: 'absent',
    complexDocumentResources: 'absent',
  });

  const sampleValidPageFacts = () => ({
    schemaVersion: '1.0.0',
    pageIndex: 0,
    text: 'present',
    images: 'absent',
    vectorPaths: 'absent',
    formXobjects: 'absent',
    annotations: 'absent',
    linkAnnotations: 'absent',
    widgets: 'absent',
    transparency: 'absent',
    optionalContent: 'absent',
    complexResources: 'absent',
    structureAssociation: 'absent',
    pageActions: 'absent',
    spotColors: 'absent',
    complexColorSpaces: 'absent',
    geometryProfile: 'standard_effective_box_v1',
    rotation: 0,
  });

  const sampleValidGeometry = () => ({
    pageIndex: 0,
    mediaBox: { left: 0, bottom: 0, width: 612, height: 792 },
    cropBox: null,
    effectiveBox: { left: 0, bottom: 0, width: 612, height: 792 },
    rotation: 0,
  });

  const sampleValidLosses = () => ({
    schemaVersion: '1.0.0',
    textSelectionLost: true,
    searchabilityLost: true,
    accessibilityTagsLost: false,
    linksLost: false,
    annotationsLost: false,
    formsLost: false,
    vectorQualityLost: false,
    colorSpaceFidelityRisk: false,
    spotColorsLost: false,
    layersLost: false,
    structureAssociationLost: false,
    pageActionsLost: false,
  });

  const sampleValidCanonicalRegion = () => ({
    regionId: 'reg1',
    sourceRegionIds: ['src_reg1'],
    pageIndex: 0,
    originalNormalizedRect: { x: 0.1, y: 0.1, width: 0.3, height: 0.2 },
    basePdfPointRect: { left: 61.2, bottom: 554.4, width: 183.6, height: 158.4 },
    requestedSafetyMarginPoints: 2.0,
    appliedPdfPointRect: { left: 59.2, bottom: 552.4, width: 187.6, height: 162.4 },
    appliedMargins: { left: 2.0, right: 2.0, top: 2.0, bottom: 2.0 },
    boundaryAdjusted: false,
    devicePixelRect: null,
  });

  const sampleValidPagePlan = () => {
    const geom = sampleValidGeometry();
    const geomDigest = computeGeometryDigest(geom);
    return {
      schemaVersion: '1.0.0',
      pageIndex: 0,
      strategy: 'rasterize_page',
      geometry: geom,
      geometryDigest: geomDigest,
      canonicalRegions: [sampleValidCanonicalRegion()],
      facts: sampleValidPageFacts(),
      featureLosses: sampleValidLosses(),
      blockers: [],
      warnings: [],
      executable: true,
    };
  };

  const sampleValidAckRequirements = () => ({
    schemaVersion: '1.0.0',
    rasterizedPagesRequired: true,
    textSearchLossRequired: false,
    accessibilityLossRequired: false,
    annotationLossRequired: false,
    formLossRequired: false,
    signatureInvalidationRequired: false,
    encryptionChangeRequired: false,
  });

  const sampleValidPlan = () => {
    const pagePlan = sampleValidPagePlan();
    const policy = sampleValidPolicy();
    const acks = sampleValidAcknowledgements();
    const ackReqs = sampleValidAckRequirements();
    const facts = sampleValidDocumentFacts();
    const canonicalRegions = [sampleValidCanonicalRegion()];
    const losses = sampleValidLosses();
    const inputDigest = sampleSha256;
    const contentGeneration = 0;
    const blockers = [];
    const warnings = [];
    const executable = true;
    const acknowledgementsSatisfied = true;
    const readyForExecution = true;

    const planDigest = computePlanDigest({
      inputDigest,
      contentGeneration,
      policy,
      acknowledgements: acks,
      acknowledgementRequirements: ackReqs,
      facts,
      pagePlans: [pagePlan],
      canonicalRegions,
      losses,
      blockers,
      warnings,
      executable,
      acknowledgementsSatisfied,
      readyForExecution,
    });

    return {
      schemaVersion: '1.0.0',
      planVersion: 'secure_redaction_plan_v1',
      inputDigest,
      contentGeneration,
      planDigest,
      policy,
      acknowledgements: acks,
      acknowledgementRequirements: ackReqs,
      pagePlans: [pagePlan],
      canonicalRegions,
      facts,
      losses,
      blockers,
      warnings,
      executable,
      acknowledgementsSatisfied,
      readyForExecution,
    };
  };

  const sampleValidVerificationCheck = (code = 'CHECK_1', status = 'passed') => ({
    schemaVersion: '1.0.0',
    code,
    status,
    message: 'Check passed successfully',
    details: null,
  });

  const sampleValidPublicationReport = () => ({
    schemaVersion: '1.0.0',
    publicationId: 'pub_1',
    publishedPath: '/published.pdf',
    publishedDigest: sampleSha256,
  });

  // -------------------------------------------------------------------------
  // 1. Backend Wire Format & Fixture Normalization
  // -------------------------------------------------------------------------
  describe('1. Wire Format & Fixture Normalization', () => {
    it('normalizes the exact Rust secure-redaction wire fixture', () => {
      const fixturePath = path.resolve(__dirname, '../../../src-tauri/tests/fixtures/contracts/secure-redaction-plan-wire-v1.json');
      const fixtureRaw = JSON.parse(fs.readFileSync(fixturePath, 'utf-8'));

      const normalized = normalizeSecureRedactionPlan(fixtureRaw);
      const fixtureCanonicalRegion = {
        regionId: 'canon_r1',
        sourceRegionIds: ['s1'],
        pageIndex: 0,
        originalNormalizedRect: { x: 0.1, y: 0.1, width: 0.3, height: 0.2 },
        basePdfPointRect: { left: 61.2, bottom: 554.4, width: 183.6, height: 158.4 },
        requestedSafetyMarginPoints: 2,
        appliedPdfPointRect: { left: 59.2, bottom: 552.4, width: 187.6, height: 162.4 },
        appliedMargins: { left: 2, right: 2, top: 2, bottom: 2 },
        boundaryAdjusted: false,
        devicePixelRect: null,
      };
      const expected = {
        schemaVersion: '1.0.0',
        planVersion: 'secure_redaction_plan_v1',
        inputDigest: 'a0'.repeat(32),
        contentGeneration: 0,
        planDigest: 'dffeeb396eda5448bc37a9901e02a42cd6c74eeb9f7db559d7b67642dbbd2cd3',
        policy: { schemaVersion: '1.0.0', profile: 'secure_rasterized_pages_v1', strategy: 'rasterized_pages', rasterDpi: 300, appearance: { kind: 'solid', color: 'black', opacity: 1 }, safetyMarginPoints: 2, interactiveContentPolicy: 'block', signaturePolicy: 'block', encryptionPolicy: 'block' },
        acknowledgements: sampleValidAcknowledgements(),
        acknowledgementRequirements: {
          schemaVersion: '1.0.0',
          rasterizedPagesRequired: true,
          textSearchLossRequired: true,
          accessibilityLossRequired: false,
          annotationLossRequired: false,
          formLossRequired: false,
          signatureInvalidationRequired: false,
          encryptionChangeRequired: false,
        },
        pagePlans: [{
          schemaVersion: '1.0.0', pageIndex: 0, strategy: 'rasterize_page',
          geometry: { pageIndex: 0, mediaBox: { left: 0, bottom: 0, width: 612, height: 792 }, cropBox: null, effectiveBox: { left: 0, bottom: 0, width: 612, height: 792 }, rotation: 0 },
          geometryDigest: 'f69551ae8e69502f7ba24758c0790270ccfb798fcf4fbffa886ebd67a4ce4225',
          canonicalRegions: [fixtureCanonicalRegion],
          facts: sampleValidPageFacts(), featureLosses: sampleValidLosses(), blockers: [], warnings: [], executable: true,
        }],
        canonicalRegions: [fixtureCanonicalRegion],
        facts: sampleValidDocumentFacts(), losses: sampleValidLosses(), blockers: [], warnings: [], executable: true,
        acknowledgementsSatisfied: true,
        readyForExecution: true,
      };
      expect(normalized).toEqual(expected);
      expect(validateSecureRedactionPlan(fixtureRaw)).toBe(true);

      for (const mutate of [
        (value) => { value.unexpected = true; },
        (value) => { value.inputDigest = value.input_digest; },
        (value) => { delete value.input_digest; delete value.document_digest; },
        (value) => { value.policy.profile = 'unknown_profile'; },
      ]) {
        const mutated = JSON.parse(JSON.stringify(fixtureRaw));
        mutate(mutated);
        expect(() => normalizeSecureRedactionPlan(mutated)).toThrow(RedactionError);
      }
    });

    it('rejects ambiguous alias conflict when both camelCase and snake_case properties are present', () => {
      const conflictPayload = {
        ...sampleValidPlan(),
        document_digest: sampleSha256, // conflict with documentDigest
      };
      expect(() => normalizeSecureRedactionPlan(conflictPayload)).toThrowError(/Ambiguous alias conflict/);
    });
  });

  // -------------------------------------------------------------------------
  // 2. Behavioral Enum Parity
  // -------------------------------------------------------------------------
  describe('2. Behavioral Enum Parity', () => {
    it('verifies manifest profile, document strategies, page strategies, rotations and verification statuses', () => {
      const policy = sampleValidPolicy();
      expect(() => normalizeSecureRedactionPolicy({ ...policy, profile: 'unknown_profile' })).toThrow(RedactionError);
      expect(() => normalizeSecureRedactionPolicy({ ...policy, strategy: 'unknown_strategy' })).toThrow(RedactionError);
      expect(() => normalizeSecureRedactionPolicy({ ...policy, interactiveContentPolicy: 'unknown_policy' })).toThrow(RedactionError);
      expect(() => normalizeSecureRedactionPolicy({ ...policy, signaturePolicy: 'unknown_policy' })).toThrow(RedactionError);
      expect(() => normalizeSecureRedactionPolicy({ ...policy, encryptionPolicy: 'unknown_policy' })).toThrow(RedactionError);

      const pagePlan = sampleValidPagePlan();
      expect(() => normalizePageRedactionPlan({ ...pagePlan, strategy: 'unknown_page_strategy' })).toThrow(RedactionError);

      const geom = sampleValidGeometry();
      expect(() => normalizeCanonicalPageGeometry({ ...geom, rotation: 45 })).toThrow(RedactionError);

      const check = sampleValidVerificationCheck();
      expect(() => normalizeRedactionVerificationCheck({ ...check, status: 'unknown_status' })).toThrow(RedactionError);
    });

    it('validates interactive content policies (strip and block)', () => {
      const policyStrip = { ...sampleValidPolicy(), interactiveContentPolicy: 'strip' };
      expect(() => normalizeSecureRedactionPolicy(policyStrip)).not.toThrow();

      const policyBlock = { ...sampleValidPolicy(), interactiveContentPolicy: 'block' };
      expect(() => normalizeSecureRedactionPolicy(policyBlock)).not.toThrow();
    });
  });

  // -------------------------------------------------------------------------
  // 3. Numeric Region Validation & Boundary Cases
  // -------------------------------------------------------------------------
  describe('3. Numeric Region Boundaries & Non-Finite Rejection', () => {
    it('rejects non-finite rect coordinates (NaN, Infinity, -Infinity)', () => {
      [NaN, Infinity, -Infinity].forEach(badVal => {
        expect(() => validateNormalizedTopLeftRect({ x: badVal, y: 0, width: 0.5, height: 0.5 })).toThrow(RedactionError);
        expect(() => validateNormalizedTopLeftRect({ x: 0, y: badVal, width: 0.5, height: 0.5 })).toThrow(RedactionError);
        expect(() => validateNormalizedTopLeftRect({ x: 0, y: 0, width: badVal, height: 0.5 })).toThrow(RedactionError);
        expect(() => validateNormalizedTopLeftRect({ x: 0, y: 0, width: 0.5, height: badVal })).toThrow(RedactionError);
      });
    });

    it('rejects zero and negative width/height', () => {
      expect(() => validateNormalizedTopLeftRect({ x: 0, y: 0, width: 0, height: 0.5 })).toThrow(RedactionError);
      expect(() => validateNormalizedTopLeftRect({ x: 0, y: 0, width: -0.1, height: 0.5 })).toThrow(RedactionError);
      expect(() => validateNormalizedTopLeftRect({ x: 0, y: 0, width: 0.5, height: 0 })).toThrow(RedactionError);
      expect(() => validateNormalizedTopLeftRect({ x: 0, y: 0, width: 0.5, height: -0.1 })).toThrow(RedactionError);
    });

    it('accepts tiny positive float dimensions (e.g. 1e-8)', () => {
      expect(() => validateNormalizedTopLeftRect({ x: 0, y: 0, width: 1e-8, height: 1e-8 })).not.toThrow();
    });

    it('handles exact right and bottom boundaries without false rejection', () => {
      expect(() => validateNormalizedTopLeftRect({ x: 0, y: 0, width: 1.0, height: 1.0 })).not.toThrow();
      expect(() => validateNormalizedTopLeftRect({ x: 0.5, y: 0.5, width: 0.5, height: 0.5 })).not.toThrow();
    });

    it('rejects slightly exceeding right or bottom boundary', () => {
      expect(() => validateNormalizedTopLeftRect({ x: 0.5, y: 0, width: 0.500001, height: 0.5 })).toThrow(RedactionError);
      expect(() => validateNormalizedTopLeftRect({ x: 0, y: 0.5, width: 0.5, height: 0.500001 })).toThrow(RedactionError);
    });
  });

  // -------------------------------------------------------------------------
  // 4. Strict Appearance Validation
  // -------------------------------------------------------------------------
  describe('4. Appearance Validation', () => {
    it('accepts the canonical solid black fully opaque appearance', () => {
      expect(normalizeSecureRedactionPolicy(sampleValidPolicy()).appearance).toEqual({
        kind: 'solid', color: 'black', opacity: 1.0,
      });
    });

    it('rejects missing appearance fields and all non-canonical opacity values with typed errors', () => {
      const policy = sampleValidPolicy();
      for (const field of ['kind', 'color', 'opacity']) {
        const appearance = { ...policy.appearance };
        delete appearance[field];
        expect(() => normalizeSecureRedactionPolicy({ ...policy, appearance })).toThrow(RedactionError);
      }
      for (const opacity of [NaN, Infinity, -Infinity, 0.999999, 1.000001, '1', true, null]) {
        expect(() => normalizeSecureRedactionPolicy({ ...policy, appearance: { kind: 'solid', color: 'black', opacity } })).toThrow(RedactionError);
      }
    });

    it('rejects forbidden appearance variants with typed errors', () => {
      const policy = sampleValidPolicy();
      for (const appearance of [
        { kind: 'pattern', color: 'black', opacity: 1.0 },
        { kind: 'gradient', color: 'black', opacity: 1.0 },
        { kind: 'solid', color: 'red', opacity: 1.0 },
        { kind: 'solid', color: 'black', opacity: 1.0, overlayText: 'REDACTED' },
        { kind: 'solid', color: 'black', opacity: 1.0, image: 'data:image/png;base64,...' },
      ]) {
        expect(() => normalizeSecureRedactionPolicy({ ...policy, appearance })).toThrow(RedactionError);
      }
    });

    it('rejects unknown nested appearance fields', () => {
      const policy = sampleValidPolicy();
      const badAppPolicy = {
        ...policy,
        appearance: { kind: 'solid', color: 'black', opacity: 1.0, extra: 'forbidden' },
      };
      expect(() => normalizeSecureRedactionPolicy(badAppPolicy)).toThrowError(/Unknown field/);
    });
  });

  // -------------------------------------------------------------------------
  // 5. Region Constraints & ID Validation
  // -------------------------------------------------------------------------
  describe('5. Region Constraints', () => {
    it('rejects whitespace-only or oversized region IDs and sources', () => {
      expect(() => normalizeRedactionRegionInput(sampleValidInput('   '))).toThrow(RedactionError);
      expect(() => normalizeRedactionRegionInput(sampleValidInput('a'.repeat(129)))).toThrow(RedactionError);
      expect(() => normalizeRedactionRegionInput({ ...sampleValidInput(), source: '   ' })).toThrow(RedactionError);
      expect(() => normalizeRedactionRegionInput({ ...sampleValidInput(), source: 's'.repeat(257) })).toThrow(RedactionError);
    });

    it('rejects duplicate source region IDs within canonical region', () => {
      const badCanonRegion = {
        ...sampleValidCanonicalRegion(),
        sourceRegionIds: ['src1', 'src1'],
      };
      expect(() => normalizeCanonicalRedactionRegion(badCanonRegion)).toThrow(RedactionError);
    });

    it('rejects sourceRegionIds exceeding limit', () => {
      const badCanonRegion = {
        ...sampleValidCanonicalRegion(),
        sourceRegionIds: Array.from({ length: 101 }, (_, i) => `src_${i}`),
      };
      expect(() => normalizeCanonicalRedactionRegion(badCanonRegion)).toThrow(RedactionError);
    });
  });

  // -------------------------------------------------------------------------
  // 6. Acknowledgements & Policy Blockers
  // -------------------------------------------------------------------------
  describe('6. Acknowledgements & Blocker Isolation', () => {
    it('rejects non-boolean acknowledgements (coercion rejection)', () => {
      const ack = sampleValidAcknowledgements();
      expect(() => normalizeRedactionAcknowledgements({ ...ack, rasterizedPagesAccepted: 'false' })).toThrow(RedactionError);
      expect(() => normalizeRedactionAcknowledgements({ ...ack, rasterizedPagesAccepted: 1 })).toThrow(RedactionError);
    });

    it('encryption/signature acknowledgements cannot bypass policy blockers for encrypted/signed documents', () => {
      const encryptedFacts = { ...sampleValidDocumentFacts(), isEncrypted: true };
      const ack = sampleValidAcknowledgements(); // encryptionChangeAccepted: true
      const blockers = evaluateDocumentBlockers(encryptedFacts, sampleValidPolicy(), ack);
      expect(blockers.length).toBe(1);
      expect(blockers[0].code).toBe('REDACTION_ENCRYPTED_INPUT_UNSUPPORTED');

      const signedFacts = { ...sampleValidDocumentFacts(), isSigned: true };
      const signedBlockers = evaluateDocumentBlockers(signedFacts, sampleValidPolicy(), ack);
      expect(signedBlockers.length).toBe(1);
      expect(signedBlockers[0].code).toBe('REDACTION_SIGNED_INPUT_UNSUPPORTED');
    });

    it('generates XFA and AcroForm blockers for unhandled dynamic/interactive forms', () => {
      const xfaFacts = { ...sampleValidDocumentFacts(), hasXfa: true };
      const xfaBlockers = evaluateDocumentBlockers(xfaFacts, sampleValidPolicy(), sampleValidAcknowledgements());
      expect(xfaBlockers.some(b => b.code === 'REDACTION_XFA_INPUT_UNSUPPORTED')).toBe(true);

      const acroFacts = { ...sampleValidDocumentFacts(), hasAcroform: true };
      const acroBlockers = evaluateDocumentBlockers(acroFacts, sampleValidPolicy(), sampleValidAcknowledgements());
      expect(acroBlockers.some(b => b.code === 'REDACTION_BLOCKED_DOCUMENT_FEATURE')).toBe(true);
    });
  });

  // -------------------------------------------------------------------------
  // 7. Collections & Duplicate Code Policy
  // -------------------------------------------------------------------------
  describe('7. Collections & Duplicate Code Policy', () => {
    it('rejects duplicate blocker, warning, or check codes in collections', () => {
      const b1 = { schemaVersion: '1.0.0', code: 'CODE_1', message: 'msg1', details: null };
      const b2 = { schemaVersion: '1.0.0', code: 'CODE_1', message: 'msg2', details: null };
      expect(() => validateUniqueBlockers([b1, b2])).toThrowError(/Duplicate blocker code/);

      const w1 = { schemaVersion: '1.0.0', code: 'W_1', message: 'msg1', details: null };
      const w2 = { schemaVersion: '1.0.0', code: 'W_1', message: 'msg2', details: null };
      expect(() => validateUniqueWarnings([w1, w2])).toThrowError(/Duplicate warning code/);

      const c1 = sampleValidVerificationCheck('CHK_1');
      const c2 = sampleValidVerificationCheck('CHK_1');
      expect(() => validateUniqueVerificationChecks([c1, c2])).toThrowError(/Duplicate verification check code/);
    });

    it('enforces the blocker collection limit', () => {
      const oversizedBlockers = Array.from({ length: 101 }, (_, i) => ({
        schemaVersion: '1.0.0',
        code: `BLOCK_${i}`,
        message: 'msg',
        details: null,
      }));
      expect(() => validateUniqueBlockers(oversizedBlockers)).toThrow(RedactionError);
    });
  });

  // -------------------------------------------------------------------------
  // 8. Cross-Report Integrity
  // -------------------------------------------------------------------------
  describe('8. Cross-Report Digest & Identity Matching', () => {
    const validExecReport = () => ({
      schemaVersion: '1.0.0',
      jobId: 'job1',
      sessionId: 'sess1',
      transformReport: {
        schemaVersion: '1.0.0',
        jobId: 'job1',
        sessionId: 'sess1',
        planDigest: sampleSha256,
        processedPages: 1,
        outputPath: '/out.pdf',
        outputDigest: sampleSha256,
        warnings: [],
        status: 'transformed',
      },
      verificationReport: {
        schemaVersion: '1.0.0',
        jobId: 'job1',
        sessionId: 'sess1',
        planDigest: sampleSha256,
        transformDigest: sampleSha256,
        isVerified: true,
        verificationChecks: [sampleValidVerificationCheck()],
        blockers: [],
        warnings: [],
      },
      published: true,
      publicationReport: sampleValidPublicationReport(),
    });

    it('rejects job_id or session_id mismatch across sub-reports', () => {
      const rep = validExecReport();
      rep.transformReport.jobId = 'diff_job';
      expect(() => normalizeSecureRedactionExecutionReport(rep)).toThrow(RedactionError);

      const repSess = validExecReport();
      repSess.verificationReport.sessionId = 'diff_sess';
      expect(() => normalizeSecureRedactionExecutionReport(repSess)).toThrow(RedactionError);
    });

    it('rejects digest mismatches across transform, verification, and publication reports', () => {
      const diffDigest = 'b'.repeat(64);
      const repPlan = validExecReport();
      repPlan.verificationReport.planDigest = diffDigest;
      expect(() => normalizeSecureRedactionExecutionReport(repPlan)).toThrow(RedactionError);

      const repTrans = validExecReport();
      repTrans.verificationReport.transformDigest = diffDigest;
      expect(() => normalizeSecureRedactionExecutionReport(repTrans)).toThrow(RedactionError);

      const repPub = validExecReport();
      repPub.publicationReport.publishedDigest = diffDigest;
      expect(() => normalizeSecureRedactionExecutionReport(repPub)).toThrow(RedactionError);
    });
  });

  // -------------------------------------------------------------------------
  // 9. Page Plan Invariants
  // -------------------------------------------------------------------------
  describe('9. Page Plan Invariants & Geometry Matching', () => {
    it('requires mandatory strategy, geometry, geometryDigest, and executable fields', () => {
      for (const field of ['strategy', 'geometry', 'geometryDigest', 'executable']) {
        const pagePlan = sampleValidPagePlan();
        delete pagePlan[field];
        expect(() => normalizePageRedactionPlan(pagePlan)).toThrow(RedactionError);
      }
    });

    it('rejects invalid page-plan strategy, geometry, geometry digest, and executable values', () => {
      expect(() => normalizePageRedactionPlan({ ...sampleValidPagePlan(), strategy: 'unknown' })).toThrow(RedactionError);
      for (const geometry of [null, false, { pageIndex: 0 }]) {
        expect(() => normalizePageRedactionPlan({ ...sampleValidPagePlan(), geometry })).toThrow(RedactionError);
      }
      for (const geometryDigest of ['', 'A'.repeat(64)]) {
        expect(() => normalizePageRedactionPlan({ ...sampleValidPagePlan(), geometryDigest })).toThrow(RedactionError);
      }
      for (const executable of ['true', 1]) {
        expect(() => normalizePageRedactionPlan({ ...sampleValidPagePlan(), executable })).toThrow(RedactionError);
      }
    });

    it('rejects pageIndex mismatch between plan, geometry, facts, and regions', () => {
      const pagePlanGeom = sampleValidPagePlan();
      pagePlanGeom.geometry.pageIndex = 1;
      expect(() => normalizePageRedactionPlan(pagePlanGeom)).toThrow(RedactionError);

      const pagePlanFacts = sampleValidPagePlan();
      pagePlanFacts.facts.pageIndex = 1;
      expect(() => normalizePageRedactionPlan(pagePlanFacts)).toThrow(RedactionError);

      const pagePlanReg = sampleValidPagePlan();
      pagePlanReg.canonicalRegions[0].pageIndex = 1;
      expect(() => normalizePageRedactionPlan(pagePlanReg)).toThrow(RedactionError);
    });
  });

  // -------------------------------------------------------------------------
  // 10. Optional Strictness (No Coercion of False/0/"")
  // -------------------------------------------------------------------------
  describe('10. Optional Strictness', () => {
    it('accepts documented null and omitted values on their owning DTOs', () => {
      const geometryWithoutCropBox = sampleValidGeometry();
      delete geometryWithoutCropBox.cropBox;
      expect(normalizeCanonicalPageGeometry(geometryWithoutCropBox).cropBox).toBeNull();
      expect(normalizeCanonicalPageGeometry({ ...sampleValidGeometry(), cropBox: null }).cropBox).toBeNull();
      const regionWithoutDevicePixelRect = sampleValidCanonicalRegion();
      delete regionWithoutDevicePixelRect.devicePixelRect;
      expect(normalizeCanonicalRedactionRegion(regionWithoutDevicePixelRect).devicePixelRect).toBeNull();
      expect(normalizeCanonicalRedactionRegion({ ...sampleValidCanonicalRegion(), devicePixelRect: null }).devicePixelRect).toBeNull();
      expect(normalizeRedactionBlocker({ schemaVersion: '1.0.0', code: 'C1', message: 'M1', details: null }).details).toBeNull();
      expect(normalizeRedactionBlocker({ schemaVersion: '1.0.0', code: 'C1', message: 'M1' }).details).toBeNull();
    });

    it('rejects false, zero, empty strings, and objects at optional object/string DTO fields', () => {
      const geom = sampleValidGeometry();
      for (const cropBox of [false, 0, '']) expect(() => normalizeCanonicalPageGeometry({ ...geom, cropBox })).toThrow(RedactionError);

      const region = sampleValidCanonicalRegion();
      for (const devicePixelRect of [false, 0, '']) expect(() => normalizeCanonicalRedactionRegion({ ...region, devicePixelRect })).toThrow(RedactionError);

      for (const details of [false, 0, {}]) {
        expect(() => normalizeRedactionBlocker({ schemaVersion: '1.0.0', code: 'C1', message: 'M1', details })).toThrow(RedactionError);
      }
    });
  });

  // -------------------------------------------------------------------------
  // 11. Manifest Integrity Test
  // -------------------------------------------------------------------------
  describe('11. Manifest Parity Integrity', () => {
    it('verifies 100% parity between JS constants and secure-redaction-contract-v1.json', () => {
      const manifestPath = path.resolve(__dirname, '../../../src-tauri/tests/fixtures/contracts/secure-redaction-contract-v1.json');
      const manifestRaw = fs.readFileSync(manifestPath, 'utf-8');
      const manifest = JSON.parse(manifestRaw);

      expect(REDACTION_SCHEMA_VERSION).toBe(manifest.schemaVersion);
      expect(manifest.wireFormat).toBe('snake_case');

      for (const [key, value] of Object.entries(manifest.limits)) {
        expect(REDACTION_LIMITS[key], `JS limit key '${key}' mismatch with manifest`).toBe(value);
      }

      const manifestErrorCodes = new Set(manifest.errorCodes);
      const jsErrorCodes = new Set(Object.values(REDACTION_ERROR_CODES));
      expect(jsErrorCodes).toEqual(manifestErrorCodes);

      expect(U32_MIN).toBe(manifest.numericRanges.u32Min);
      expect(U32_MAX).toBe(manifest.numericRanges.u32Max);
      expect(I32_MIN).toBe(manifest.numericRanges.i32Min);
      expect(I32_MAX).toBe(manifest.numericRanges.i32Max);
    });
  });

  describe('12. Manifest runtime enum, policy, digest, and numeric parity', () => {
    const manifest = JSON.parse(fs.readFileSync(path.resolve(__dirname, '../../../src-tauri/tests/fixtures/contracts/secure-redaction-contract-v1.json'), 'utf-8'));

    it('accepts every manifest enum value through the owning validator and rejects an unknown value', () => {
      expect(normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), profile: manifest.profile }).profile).toBe(manifest.profile);
      for (const strategy of manifest.documentStrategies) {
        expect(normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), strategy }).strategy).toBe(strategy);
      }
      for (const strategy of manifest.pageStrategies) {
        const canonicalRegions = strategy === 'import_unaffected_page' ? [] : [sampleValidCanonicalRegion()];
        expect(normalizePageRedactionPlan({ ...sampleValidPagePlan(), strategy, canonicalRegions }).strategy).toBe(strategy);
      }
      for (const rotation of manifest.rotations) {
        expect(normalizeCanonicalPageGeometry({ ...sampleValidGeometry(), rotation }).rotation).toBe(rotation);
        expect(normalizePageRedactionFacts({ ...sampleValidPageFacts(), rotation }).rotation).toBe(rotation);
      }
      for (const status of manifest.verificationStatuses) {
        expect(normalizeRedactionVerificationCheck({ ...sampleValidVerificationCheck(), status }).status).toBe(status);
      }
      for (const kind of manifest.appearance.kinds) {
        for (const color of manifest.appearance.colors) {
          for (const opacity of manifest.appearance.opacity) {
            expect(normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), appearance: { kind, color, opacity } }).appearance).toEqual({ kind, color, opacity });
          }
        }
      }
      for (const interactiveContentPolicy of manifest.interactiveContentPolicies) {
        expect(normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), interactiveContentPolicy }).interactiveContentPolicy).toBe(interactiveContentPolicy);
      }
      for (const signaturePolicy of manifest.signaturePolicies) {
        expect(normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), signaturePolicy }).signaturePolicy).toBe(signaturePolicy);
      }
      for (const encryptionPolicy of manifest.encryptionPolicies) {
        expect(normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), encryptionPolicy }).encryptionPolicy).toBe(encryptionPolicy);
      }
      expect(() => normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), profile: 'unknown' })).toThrow(RedactionError);
      expect(() => normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), strategy: 'unknown' })).toThrow(RedactionError);
      expect(() => normalizePageRedactionPlan({ ...sampleValidPagePlan(), strategy: 'unknown' })).toThrow(RedactionError);
      expect(() => normalizeCanonicalPageGeometry({ ...sampleValidGeometry(), rotation: 45 })).toThrow(RedactionError);
      expect(() => normalizeRedactionVerificationCheck({ ...sampleValidVerificationCheck(), status: 'unknown' })).toThrow(RedactionError);
      expect(() => normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), interactiveContentPolicy: 'unknown' })).toThrow(RedactionError);
      expect(() => normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), signaturePolicy: 'unknown' })).toThrow(RedactionError);
      expect(() => normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), encryptionPolicy: 'unknown' })).toThrow(RedactionError);
    });

    it('ties digest rules and JavaScript error-code set exactly to the manifest', () => {
      expect(manifest.digest).toEqual({ algorithm: 'sha256', encoding: 'lowercase_hex', length: 64 });
      expect(new Set(Object.values(REDACTION_ERROR_CODES))).toEqual(new Set(manifest.errorCodes));
      expect(Object.values(REDACTION_ERROR_CODES)).toHaveLength(15);
      expect(new Set(Object.values(REDACTION_ERROR_CODES)).size).toBe(15);
      for (const invalid of ['a'.repeat(63), 'a'.repeat(65), 'A'.repeat(64), `${'a'.repeat(63)}g`, ` ${'a'.repeat(63)}`, `${'a'.repeat(63)} `, `${'a'.repeat(31)} ${'a'.repeat(32)}`, '', 1, {}, null]) {
        expect(() => validateSha256Digest(invalid, 'digest')).toThrow(RedactionError);
      }
      expect(() => validateSha256Digest('a'.repeat(manifest.digest.length), 'digest')).not.toThrow();
    });

    it('uses manifest u32 and i32 ranges for page indexes and device pixels', () => {
      const { u32Min, u32Max, i32Min, i32Max } = manifest.numericRanges;
      expect(assertU32(u32Min, 'pageIndex')).toBe(u32Min);
      expect(assertU32(u32Max, 'pageIndex')).toBe(u32Max);
      for (const invalid of [u32Min - 1, u32Max + 1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
        expect(() => assertU32(invalid, 'pageIndex')).toThrow(RedactionError);
      }
      expect(normalizeDevicePixelRect({ left: i32Min, top: 0, width: 1, height: 1 }).left).toBe(i32Min);
      expect(normalizeDevicePixelRect({ left: 0, top: i32Max - 1, width: 1, height: 1 }).top).toBe(i32Max - 1);
      for (const invalid of [i32Min - 1, i32Max + 1, 1.25, Number.MAX_SAFE_INTEGER + 1]) {
        expect(() => normalizeDevicePixelRect({ left: invalid, top: 0, width: 1, height: 1 })).toThrow(RedactionError);
      }
    });
  });

  describe('13. Active limit and UTF-8 byte boundaries', () => {
    const text = (bytes) => 'a'.repeat(bytes);
    const unique = (count, prefix) => Array.from({ length: count }, (_, index) => `${prefix}${index}`);

    it('enforces exact collection boundaries for regions, canonical regions, page plans, blockers, warnings, and checks', () => {
      expect(validateRedactionRegionInputs(unique(REDACTION_LIMITS.MAX_REGIONS_PER_INPUT, 'r').map((id) => sampleValidInput(id)))).toHaveLength(REDACTION_LIMITS.MAX_REGIONS_PER_INPUT);
      expect(() => validateRedactionRegionInputs(unique(REDACTION_LIMITS.MAX_REGIONS_PER_INPUT + 1, 'r').map((id) => sampleValidInput(id)))).toThrow(RedactionError);
      expect(normalizeCanonicalRedactionRegion({ ...sampleValidCanonicalRegion(), sourceRegionIds: unique(REDACTION_LIMITS.MAX_SOURCE_REGION_IDS, 's') }).sourceRegionIds).toHaveLength(REDACTION_LIMITS.MAX_SOURCE_REGION_IDS);
      expect(() => normalizeCanonicalRedactionRegion({ ...sampleValidCanonicalRegion(), sourceRegionIds: unique(REDACTION_LIMITS.MAX_SOURCE_REGION_IDS + 1, 's') })).toThrow(RedactionError);
      expect(normalizePageRedactionPlan({ ...sampleValidPagePlan(), canonicalRegions: Array.from({ length: REDACTION_LIMITS.MAX_CANONICAL_REGIONS_PER_PAGE }, (_, index) => ({ ...sampleValidCanonicalRegion(), regionId: `r${String(index).padStart(5, '0')}` })) }).canonicalRegions).toHaveLength(REDACTION_LIMITS.MAX_CANONICAL_REGIONS_PER_PAGE);
      expect(() => normalizePageRedactionPlan({ ...sampleValidPagePlan(), canonicalRegions: Array.from({ length: REDACTION_LIMITS.MAX_CANONICAL_REGIONS_PER_PAGE + 1 }, (_, index) => ({ ...sampleValidCanonicalRegion(), regionId: `r${String(index).padStart(5, '0')}` })) })).toThrow(RedactionError);
      const planPages = (count) => Array.from({ length: count }, (_, pageIndex) => {
        const geom = { ...sampleValidGeometry(), pageIndex };
        const geomDigest = computeGeometryDigest(geom);
        const canonReg = { ...sampleValidCanonicalRegion(), pageIndex };
        return {
          ...sampleValidPagePlan(),
          pageIndex,
          geometry: geom,
          geometryDigest: geomDigest,
          facts: { ...sampleValidPageFacts(), pageIndex },
          canonicalRegions: [canonReg],
        };
      });
      const pagesMax = planPages(REDACTION_LIMITS.MAX_PAGE_PLANS);
      const regionsMax = pagesMax.flatMap(p => p.canonicalRegions);
      const rawPlanMax = { ...sampleValidPlan(), pagePlans: pagesMax, canonicalRegions: regionsMax, facts: { ...sampleValidDocumentFacts(), totalPages: REDACTION_LIMITS.MAX_PAGE_PLANS } };
      rawPlanMax.planDigest = computePlanDigest(rawPlanMax);
      expect(normalizeSecureRedactionPlan(rawPlanMax).pagePlans).toHaveLength(REDACTION_LIMITS.MAX_PAGE_PLANS);
      const pagesOver = planPages(REDACTION_LIMITS.MAX_PAGE_PLANS + 1);
      const regionsOver = pagesOver.flatMap(p => p.canonicalRegions);
      const rawPlanOver = { ...sampleValidPlan(), pagePlans: pagesOver, canonicalRegions: regionsOver, facts: { ...sampleValidDocumentFacts(), totalPages: REDACTION_LIMITS.MAX_PAGE_PLANS + 1 } };
      rawPlanOver.planDigest = computePlanDigest(rawPlanOver);
      expect(() => normalizeSecureRedactionPlan(rawPlanOver)).toThrow(RedactionError);
      expect(validateUniqueBlockers(unique(REDACTION_LIMITS.MAX_BLOCKERS, 'b').map((code) => ({ schemaVersion: '1.0.0', code, message: 'm', details: null })))).toHaveLength(REDACTION_LIMITS.MAX_BLOCKERS);
      expect(() => validateUniqueBlockers(unique(REDACTION_LIMITS.MAX_BLOCKERS + 1, 'b').map((code) => ({ schemaVersion: '1.0.0', code, message: 'm', details: null })))).toThrow(RedactionError);
      expect(validateUniqueWarnings(unique(REDACTION_LIMITS.MAX_WARNINGS, 'w').map((code) => ({ schemaVersion: '1.0.0', code, message: 'm', details: null })))).toHaveLength(REDACTION_LIMITS.MAX_WARNINGS);
      expect(() => validateUniqueWarnings(unique(REDACTION_LIMITS.MAX_WARNINGS + 1, 'w').map((code) => ({ schemaVersion: '1.0.0', code, message: 'm', details: null })))).toThrow(RedactionError);
      expect(validateUniqueVerificationChecks(unique(REDACTION_LIMITS.MAX_VERIFICATION_CHECKS, 'c').map((code) => sampleValidVerificationCheck(code)))).toHaveLength(REDACTION_LIMITS.MAX_VERIFICATION_CHECKS);
      expect(() => validateUniqueVerificationChecks(unique(REDACTION_LIMITS.MAX_VERIFICATION_CHECKS + 1, 'c').map((code) => sampleValidVerificationCheck(code)))).toThrow(RedactionError);
    });

    it('enforces every active string, DPI, and safety-margin limit at exact UTF-8 byte boundaries', () => {
      const stringCases = [
        [REDACTION_LIMITS.MAX_REGION_ID_LEN, (value) => normalizeRedactionRegionInput(sampleValidInput(value))],
        [REDACTION_LIMITS.MAX_SOURCE_LEN, (value) => normalizeRedactionRegionInput({ ...sampleValidInput(), source: value })],
        [REDACTION_LIMITS.MAX_BLOCKER_CODE_LEN, (value) => normalizeRedactionBlocker({ schemaVersion: '1.0.0', code: value, message: 'm', details: null })],
        [REDACTION_LIMITS.MAX_WARNING_CODE_LEN, (value) => normalizeRedactionWarning({ schemaVersion: '1.0.0', code: value, message: 'm', details: null })],
        [REDACTION_LIMITS.MAX_USER_SAFE_MESSAGE_LEN, (value) => normalizeRedactionBlocker({ schemaVersion: '1.0.0', code: 'c', message: value, details: null })],
        [REDACTION_LIMITS.MAX_DETAILS_LEN, (value) => normalizeRedactionBlocker({ schemaVersion: '1.0.0', code: 'c', message: 'm', details: value })],
        [REDACTION_LIMITS.MAX_VERIFICATION_CHECK_CODE_LEN, (value) => normalizeRedactionVerificationCheck({ ...sampleValidVerificationCheck(), code: value })],
        [REDACTION_LIMITS.MAX_VERIFICATION_CHECK_MESSAGE_LEN, (value) => normalizeRedactionVerificationCheck({ ...sampleValidVerificationCheck(), message: value })],
        [REDACTION_LIMITS.MAX_VERIFICATION_CHECK_DETAILS_LEN, (value) => normalizeRedactionVerificationCheck({ ...sampleValidVerificationCheck(), details: value })],
        [REDACTION_LIMITS.MAX_IDENTITY_LEN, (value) => normalizeSecureRedactionTransformReport({ schemaVersion: '1.0.0', jobId: value, sessionId: 's', planDigest: sampleSha256, processedPages: 0, outputPath: '/x', outputDigest: sampleSha256, warnings: [], status: 'transformed' })],
        [REDACTION_LIMITS.MAX_PATH_LEN, (value) => normalizeSecureRedactionPublicationReport({ schemaVersion: '1.0.0', publicationId: 'p', publishedPath: value, publishedDigest: sampleSha256 })],
      ];
      for (const [limit, validate] of stringCases) {
        expect(() => validate(text(limit))).not.toThrow();
        expect(() => validate(text(limit + 1))).toThrow(RedactionError);
      }
      for (const value of ['a', 'é', 'ğ', '🙂', 'aéğ🙂']) expect(getUtf8ByteLength(value)).toBe(new TextEncoder().encode(value).length);
      const twoByteAtLimit = 'é'.repeat(REDACTION_LIMITS.MAX_REGION_ID_LEN / 2);
      expect(() => normalizeRedactionRegionInput(sampleValidInput(twoByteAtLimit))).not.toThrow();
      expect(() => normalizeRedactionRegionInput(sampleValidInput(`${twoByteAtLimit}a`))).toThrow(RedactionError);
      for (const rasterDpi of [REDACTION_LIMITS.MIN_RASTER_DPI, REDACTION_LIMITS.MAX_RASTER_DPI]) expect(() => normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), rasterDpi })).not.toThrow();
      for (const rasterDpi of [REDACTION_LIMITS.MIN_RASTER_DPI - 1, REDACTION_LIMITS.MAX_RASTER_DPI + 1]) expect(() => normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), rasterDpi })).toThrow(RedactionError);
      for (const safetyMarginPoints of [REDACTION_LIMITS.MIN_SAFETY_MARGIN_POINTS, REDACTION_LIMITS.MAX_SAFETY_MARGIN_POINTS]) expect(() => normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), safetyMarginPoints })).not.toThrow();
      for (const safetyMarginPoints of [REDACTION_LIMITS.MIN_SAFETY_MARGIN_POINTS - 0.001, REDACTION_LIMITS.MAX_SAFETY_MARGIN_POINTS + 0.001]) expect(() => normalizeSecureRedactionPolicy({ ...sampleValidPolicy(), safetyMarginPoints })).toThrow(RedactionError);
    });
  });

  describe('14. Unsupported schema versions fail closed for every versioned DTO', () => {
    const expectUnsupportedVersion = (normalize, value) => {
      try {
        normalize({ ...value, schemaVersion: '999.0.0' });
        throw new Error('expected unsupported schema version to be rejected');
      } catch (error) {
        expect(error).toBeInstanceOf(RedactionError);
        expect(error.code).toBe(REDACTION_ERROR_CODES.REDACTION_POLICY_VERSION_UNSUPPORTED);
      }
    };

    it('returns REDACTION_POLICY_VERSION_UNSUPPORTED for each independently versioned DTO', () => {
      const transform = { schemaVersion: '1.0.0', jobId: 'job', sessionId: 'session', planDigest: sampleSha256, processedPages: 0, outputPath: '/out.pdf', outputDigest: sampleSha256, warnings: [], status: 'transformed' };
      const verification = { schemaVersion: '1.0.0', jobId: 'job', sessionId: 'session', planDigest: sampleSha256, transformDigest: sampleSha256, isVerified: true, verificationChecks: [sampleValidVerificationCheck()], blockers: [], warnings: [] };
      const execution = { schemaVersion: '1.0.0', jobId: 'job', sessionId: 'session', transformReport: transform, verificationReport: verification, published: false, publicationReport: null };
      for (const [name, normalize, value] of [
        ['SecureRedactionPolicy', normalizeSecureRedactionPolicy, sampleValidPolicy()],
        ['RedactionRegionInput carrier root', normalizeSecureRedactionPlan, sampleValidPlan()],
        ['DocumentRedactionFacts', normalizeDocumentRedactionFacts, sampleValidDocumentFacts()],
        ['PageRedactionFacts', normalizePageRedactionFacts, sampleValidPageFacts()],
        ['RedactionFeatureLosses', normalizeRedactionFeatureLosses, sampleValidLosses()],
        ['RedactionBlocker', normalizeRedactionBlocker, { schemaVersion: '1.0.0', code: 'B', message: 'block', details: null }],
        ['RedactionWarning', normalizeRedactionWarning, { schemaVersion: '1.0.0', code: 'W', message: 'warn', details: null }],
        ['RedactionVerificationCheck', normalizeRedactionVerificationCheck, sampleValidVerificationCheck()],
        ['PageRedactionPlan', normalizePageRedactionPlan, sampleValidPagePlan()],
        ['SecureRedactionPlan', normalizeSecureRedactionPlan, sampleValidPlan()],
        ['SecureRedactionTransformReport', normalizeSecureRedactionTransformReport, transform],
        ['SecureRedactionVerificationReport', normalizeSecureRedactionVerificationReport, verification],
        ['SecureRedactionPublicationReport', normalizeSecureRedactionPublicationReport, sampleValidPublicationReport()],
        ['SecureRedactionExecutionReport', normalizeSecureRedactionExecutionReport, execution],
      ]) {
        expectUnsupportedVersion(normalize, value);
      }
    });
  });

  describe('15. Verification and publication invariants', () => {
    const validTransform = () => ({ schemaVersion: '1.0.0', jobId: 'job', sessionId: 'session', planDigest: sampleSha256, processedPages: 1, outputPath: '/out.pdf', outputDigest: sampleSha256, warnings: [], status: 'transformed' });
    const validVerification = () => ({ schemaVersion: '1.0.0', jobId: 'job', sessionId: 'session', planDigest: sampleSha256, transformDigest: sampleSha256, isVerified: true, verificationChecks: [sampleValidVerificationCheck()], blockers: [], warnings: [] });
    const validExecution = () => ({ schemaVersion: '1.0.0', jobId: 'job', sessionId: 'session', transformReport: validTransform(), verificationReport: validVerification(), published: true, publicationReport: sampleValidPublicationReport() });

    it('accepts only verified reports with checks present, passed, and unblocked', () => {
      expect(normalizeSecureRedactionVerificationReport(validVerification()).isVerified).toBe(true);
      for (const mutate of [
        (report) => { report.verificationChecks = []; },
        (report) => { report.verificationChecks[0].status = 'failed'; },
        (report) => { report.verificationChecks[0].status = 'not_run'; },
        (report) => { report.blockers = [{ schemaVersion: '1.0.0', code: 'BLOCK', message: 'blocked', details: null }]; },
      ]) {
        const report = validVerification();
        mutate(report);
        expect(() => normalizeSecureRedactionVerificationReport(report)).toThrow(RedactionError);
      }
      expect(() => normalizeRedactionVerificationCheck({ ...sampleValidVerificationCheck(), status: 'unknown' })).toThrow(RedactionError);
      expect(() => normalizeSecureRedactionVerificationReport({ ...validVerification(), verificationChecks: [sampleValidVerificationCheck('DUP'), sampleValidVerificationCheck('DUP')] })).toThrow(RedactionError);
      expect(() => normalizeSecureRedactionVerificationReport({ ...validVerification(), verificationChecks: Array.from({ length: REDACTION_LIMITS.MAX_VERIFICATION_CHECKS + 1 }, (_, index) => sampleValidVerificationCheck(`CHECK_${index}`)) })).toThrow(RedactionError);
    });

    it('accepts unpublished execution reports only with a null publication report and rejects primitive reports', () => {
      expect(normalizeSecureRedactionExecutionReport({ ...validExecution(), published: false, publicationReport: null }).publicationReport).toBeNull();
      for (const publicationReport of [sampleValidPublicationReport(), false, 0, '']) {
        expect(() => normalizeSecureRedactionExecutionReport({ ...validExecution(), published: false, publicationReport })).toThrow(RedactionError);
      }
    });

    it('requires a complete, verified, matching publication chain when published is true', () => {
      for (const mutate of [
        (report) => { report.publicationReport = null; },
        (report) => { report.publicationReport = false; },
        (report) => { report.publicationReport.publicationId = ''; },
        (report) => { report.publicationReport.publishedPath = ''; },
        (report) => { report.publicationReport.publishedDigest = 'A'.repeat(64); },
        (report) => { report.publicationReport.publishedDigest = 'b'.repeat(64); },
        (report) => { report.verificationReport.verificationChecks[0].status = 'failed'; },
        (report) => { report.verificationReport.verificationChecks[0].status = 'not_run'; },
        (report) => { report.verificationReport.blockers = [{ schemaVersion: '1.0.0', code: 'BLOCK', message: 'blocked', details: null }]; },
        (report) => { report.transformReport.jobId = 'other-job'; },
        (report) => { report.verificationReport.sessionId = 'other-session'; },
        (report) => { report.verificationReport.planDigest = 'b'.repeat(64); },
        (report) => { report.verificationReport.transformDigest = 'b'.repeat(64); },
      ]) {
        const report = validExecution();
        mutate(report);
        expect(() => normalizeSecureRedactionExecutionReport(report)).toThrow(RedactionError);
      }
    });
  });
});
