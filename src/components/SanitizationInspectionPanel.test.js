import { fireEvent, render } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";
import { createSanitizationState } from "../stores/sanitizationState.svelte.js";
import SanitizationInspectionPanel from "./SanitizationInspectionPanel.svelte";

const report = (overrides = {}) => ({
  documentFacts: {
    encrypted: false, signed: false, incrementalRevisionsDetected: false,
    xfaDetected: false, optionalContentDetected: false, richMediaDetected: false, pageCount: 1,
    ...overrides.documentFacts,
  },
  controls: [{
    controlId: "xfa", present: false, findingCount: 0,
    inspectionSupport: "supported_for_inspection", futureRemovalSupport: "unsupported",
    expectedVisualImpact: "high", expectedFunctionalImpact: "dynamic form behavior may be lost",
    blockingReason: null,
  }, ...(overrides.controls ?? [])],
  blockingFindings: overrides.blockingFindings ?? [],
});

function reviewState(value) {
  const state = createSanitizationState();
  const revision = state.beginInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 1 });
  state.applyInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 1, revision, inspection: value });
  return state;
}

describe("SanitizationInspectionPanel", () => {
  it("renders idle wording and keeps inspection disabled by default", () => {
    const { getByText } = render(SanitizationInspectionPanel, { props: { state: createSanitizationState() } });
    expect(getByText(/Henüz inspection raporu yok/)).toBeTruthy();
    expect(getByText("İncele").disabled).toBe(true);
    expect(getByText(/Inspection belgeyi değiştirmez/)).toBeTruthy();
  });

  it("renders inspecting and a bounded cancel action", async () => {
    const state = createSanitizationState();
    state.beginInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 1 });
    const onCancel = vi.fn();
    const { getByText } = render(SanitizationInspectionPanel, { props: { state, runnable: true, onCancel } });
    expect(getByText(/Snapshot worker/)).toBeTruthy();
    await fireEvent.click(getByText("İptal"));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("renders supported no-findings without making a safety claim", () => {
    const { getByText, queryByText } = render(SanitizationInspectionPanel, { props: { state: reviewState(report()) } });
    expect(getByText(/İncelenen destekli kategorilerde bulgu yok/)).toBeTruthy();
    expect(queryByText(/güvenli PDF|tamamen temiz|bütün tehditler kaldırıldı/i)).toBeNull();
  });

  it("separates blockers and document facts including encrypted, signed, history, XFA and OCG", () => {
    const value = report({
      documentFacts: {
        encrypted: true, signed: true, incrementalRevisionsDetected: true,
        xfaDetected: true, optionalContentDetected: true, richMediaDetected: true,
      },
      controls: [{
        controlId: "optional_content", present: true, findingCount: 2,
        inspectionSupport: "supported_for_inspection", futureRemovalSupport: "unsupported",
        expectedVisualImpact: "high", expectedFunctionalImpact: "layer visibility semantics may change",
        blockingReason: "Optional content desteklenmiyor.",
      }],
      blockingFindings: [{
        code: "SANITIZATION_OPTIONAL_CONTENT_UNSUPPORTED", controlId: "optional_content",
        safeMessage: "Optional content güvenli biçimde kaldırılamaz.",
      }],
    });
    const { getByText, getAllByText } = render(SanitizationInspectionPanel, { props: { state: reviewState(value) } });
    expect(getByText("Minimum profil engelleri")).toBeTruthy();
    expect(getAllByText("Tespit edildi").length).toBeGreaterThanOrEqual(6);
    expect(getByText(/kaldırma desteği henüz yok/)).toBeTruthy();
  });

  it("renders failed and cancelled terminal states", () => {
    const failed = createSanitizationState();
    let revision = failed.beginInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 1 });
    failed.failInspection({ jobId: "job-1", sessionId: "session-1", contentGeneration: 1, revision, error: new Error("typed failure") });
    const failureView = render(SanitizationInspectionPanel, { props: { state: failed } });
    expect(failureView.getByText("typed failure")).toBeTruthy();
    failureView.unmount();

    const cancelled = createSanitizationState();
    revision = cancelled.beginInspection({ jobId: "job-2", sessionId: "session-1", contentGeneration: 1 });
    cancelled.failInspection({ jobId: "job-2", sessionId: "session-1", contentGeneration: 1, revision, error: null, cancelled: true });
    const cancelledView = render(SanitizationInspectionPanel, { props: { state: cancelled } });
    expect(cancelledView.getByText("Inspection iptal edildi.")).toBeTruthy();
  });
});
