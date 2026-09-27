import { fireEvent, render } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";
import { createSanitizationState } from "../stores/sanitizationState.svelte.js";
import SanitizationPanel from "./SanitizationPanel.svelte";

const inspection = (overrides = {}) => ({
  inspectionVersion: "sanitization-inspection-v1",
  policySchemaVersion: "sanitization-policy-v1",
  controlRegistryVersion: "sanitization-controls/1",
  documentFacts: {
    encrypted: false, signed: false, incrementalRevisionsDetected: false, xfaDetected: false,
    optionalContentDetected: false, richMediaDetected: false, pageCount: 2,
    ...overrides.documentFacts,
  },
  controls: overrides.controls ?? [
    {
      controlId: "document_javascript", present: true, findingCount: 2,
      inspectionSupport: "supported_for_inspection", futureRemovalSupport: "supported",
      expectedVisualImpact: "none", expectedFunctionalImpact: "document automation stops", blockingReason: null,
    },
    {
      controlId: "optional_content", present: false, findingCount: 0,
      inspectionSupport: "supported_for_inspection", futureRemovalSupport: "unsupported",
      expectedVisualImpact: "high", expectedFunctionalImpact: "layer visibility semantics may change", blockingReason: null,
    },
  ],
  blockingFindings: overrides.blockingFindings ?? [],
});

function reviewState(value = inspection()) {
  const state = createSanitizationState();
  const revision = state.beginInspection({ jobId: "inspect", sessionId: "session", contentGeneration: 1 });
  state.applyInspection({ jobId: "inspect", sessionId: "session", contentGeneration: 1, revision, inspection: value });
  return state;
}

function renderPanel(state, props = {}) {
  return render(SanitizationPanel, {
    props: { show: true, state, runnable: true, ...props },
  });
}

describe("SanitizationPanel", () => {
  it("starts with inspection, renders loading, and exposes current-job cancellation", async () => {
    const idle = createSanitizationState();
    const onInspect = vi.fn();
    const idleView = renderPanel(idle, { onInspect });
    await Promise.resolve();
    expect(onInspect).toHaveBeenCalledTimes(1);
    idleView.unmount();

    const loading = createSanitizationState();
    loading.beginInspection({ jobId: "inspect", sessionId: "session", contentGeneration: 1 });
    const onCancel = vi.fn();
    const view = renderPanel(loading, { onCancel });
    expect(view.getByText(/immutable snapshot/)).toBeTruthy();
    await fireEvent.click(view.getByText("İptal"));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("shows facts/findings, supported selection, disabled unsupported controls, profiles and scope wording", async () => {
    const state = reviewState();
    const onPolicyChange = vi.fn();
    const view = renderPanel(state, { onPolicyChange });
    expect(view.getByText(/yalnız desteklenen PDF yapılarını tarar/)).toBeTruthy();
    expect(view.getByText("2 bulgu")).toBeTruthy();
    expect(view.getByText("Verified limited profile")).toBeTruthy();
    expect(view.getByText("Custom cleanup")).toBeTruthy();
    const unsupported = view.getByText("optional_content").closest("label").querySelector("input");
    expect(unsupported.disabled).toBe(true);
    await fireEvent.click(view.getByText("Custom cleanup"));
    expect(onPolicyChange).toHaveBeenCalledWith(expect.objectContaining({ profile: "custom_supported_v1" }));
  });

  it("keeps unsupported blockers hard-blocked and signature acknowledgement explicit/default-false", async () => {
    const blocked = reviewState(inspection({
      documentFacts: { encrypted: true, signed: true },
      blockingFindings: [
        { controlId: "encryption", code: "ENCRYPTED", safeMessage: "Encrypted input unsupported" },
        { controlId: "digital_signatures", code: "SIGNED", safeMessage: "Signature invalidation acknowledgement required" },
      ],
    }));
    const onAcknowledgementChange = vi.fn();
    const view = renderPanel(blocked, { onAcknowledgementChange });
    expect(view.getByText(/acknowledgement ile aşılamaz/)).toBeTruthy();
    expect(view.getByText(/Full rewrite mevcut dijital imzaları/)).toBeTruthy();
    const acknowledgement = view.getByText(/Full rewrite mevcut dijital imzaları/).closest("label").querySelector("input");
    expect(acknowledgement.checked).toBe(false);
    expect(view.getByText("Kaydet ve çalıştır").disabled).toBe(true);
    await fireEvent.click(acknowledgement);
    expect(onAcknowledgementChange).toHaveBeenCalledWith("signatureInvalidationAccepted", true);
  });

  it("triggers execution only from ready review and renders execution/cancel state", async () => {
    const ready = reviewState();
    const onExecute = vi.fn();
    const view = renderPanel(ready, { onExecute });
    expect(view.getByText("Kaydet ve çalıştır").disabled).toBe(false);
    await fireEvent.click(view.getByText("Kaydet ve çalıştır"));
    expect(onExecute).toHaveBeenCalledTimes(1);
    view.unmount();

    const executing = reviewState();
    executing.beginExecution("execute", { request: true });
    const onCancel = vi.fn();
    const executionView = renderPanel(executing, { onCancel });
    expect(executionView.getByText(/bağımsız doğrulama/)).toBeTruthy();
    await fireEvent.click(executionView.getByText("İptal"));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it.each([
    ["publication_conflict", "Hedef dosya işlem sırasında değişti. Mevcut dosyanın üzerine yazılmadı."],
    ["verification_failed", "Çıktı bağımsız doğrulamadan geçmedi ve kullanıcı hedefinde yayımlanmadı."],
    ["recovery_required", "Dosya sistemi sonucu kurtarma gerektiriyor."],
    ["indeterminate", "Dosya sistemindeki nihai durum kesin olarak doğrulanamadı."],
  ])("preserves %s failure semantics", (kind, message) => {
    const state = reviewState();
    state.failExecution(null, {
      kind, message, commitState: kind === "recovery_required" ? "recovery_required" : null,
      recoveryPath: kind === "recovery_required" ? "/safe/recovery.pdf" : null,
      replacementDigest: kind === "recovery_required" ? "a".repeat(64) : null,
    });
    const view = renderPanel(state);
    expect(view.getByText(message)).toBeTruthy();
    if (kind === "recovery_required") {
      expect(view.getByText(/Kurtarma yolu/)).toBeTruthy();
      expect(view.getByText(/Replacement digest/)).toBeTruthy();
    }
  });

  it.each([
    ["verified_limited", "Doğrulanmış sınırlı sanitization tamamlandı."],
    ["verified_custom", "Seçili öğelerin doğrulanmış temizliği tamamlandı."],
  ])("renders %s success without broad safety claims", (assurance, heading) => {
    const state = reviewState();
    state.beginExecution("execute", { request: true });
    state.completeExecution("execute", {
      assurance,
      profile: assurance === "verified_limited" ? "verified_limited_v1" : "custom_supported_v1",
      controls: ["document_javascript"],
      transform: { controls: [{ controlId: "document_javascript", transformStatus: "not_found" }] },
      verification: { isVerified: true },
      publication: {
        targetPath: "/exports/output.pdf", published: true, publishedDigest: "a".repeat(64),
        commitState: "new_committed_durability_unconfirmed",
      },
    });
    const view = renderPanel(state);
    expect(view.getByText(heading)).toBeTruthy();
    expect(view.getAllByText(/Bulunmadı/).length).toBeGreaterThan(0);
    expect(view.getByText(/durability onayı tamamlanamadı/)).toBeTruthy();
    expect(view.queryByText(/Tamamen temiz PDF|Bütün tehditler kaldırıldı|Güvenli PDF|Zararsız PDF|Hiçbir gizli veri kalmadı/i)).toBeNull();
  });
});
