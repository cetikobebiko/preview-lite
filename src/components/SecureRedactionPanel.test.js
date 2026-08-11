import { fireEvent, render } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";
import { createSecureRedactionState } from "../stores/secureRedactionState.svelte.js";
import SecureRedactionPanel from "./SecureRedactionPanel.svelte";

const region = () => ({
  id: "region-1",
  pageIndex: 2,
  normalizedTopLeftRect: { x: 0.1, y: 0.2, width: 0.3, height: 0.2 },
  source: "manual_selection",
});
const losses = (overrides = {}) => ({
  textSelectionLost: true,
  searchabilityLost: true,
  accessibilityTagsLost: false,
  linksLost: false,
  annotationsLost: false,
  formsLost: false,
  vectorQualityLost: true,
  colorSpaceFidelityRisk: false,
  spotColorsLost: false,
  layersLost: false,
  structureAssociationLost: false,
  pageActionsLost: false,
  ...overrides,
});
const requirements = (overrides = {}) => ({
  rasterizedPagesRequired: true,
  textSearchLossRequired: true,
  accessibilityLossRequired: false,
  annotationLossRequired: false,
  formLossRequired: false,
  signatureInvalidationRequired: false,
  encryptionChangeRequired: false,
  ...overrides,
});
const plan = (overrides = {}) => ({
  digestVerified: true,
  executable: true,
  readyForExecution: false,
  blockers: [],
  warnings: [{ code: "QUALITY", message: "Renk doğruluğunu gözden geçirin." }],
  pagePlans: [{ pageIndex: 0, strategy: "rasterize_page" }, { pageIndex: 1, strategy: "import_unaffected_page" }],
  losses: losses(),
  acknowledgementRequirements: requirements(),
  ...overrides,
});

function props(state, overrides = {}) {
  return {
    state,
    onPlan: vi.fn(), onExecute: vi.fn(), onCancelPlan: vi.fn(), onCancelExecution: vi.fn(),
    onToggleSelection: vi.fn(), onRemoveRegion: vi.fn(), onClearRegions: vi.fn(), onNavigate: vi.fn(),
    onPolicyChange: vi.fn(), onAcknowledgementChange: vi.fn(), onNewRedaction: vi.fn(),
    ...overrides,
  };
}

describe("SecureRedactionPanel", () => {
  it("renders an explicit empty pending state without secure-result wording", () => {
    const state = createSecureRedactionState();
    const { getByText, queryByText } = render(SecureRedactionPanel, { props: props(state) });
    expect(getByText(/Bekleyen adaylar PDF içeriğini kaldırmaz/)).toBeTruthy();
    expect(getByText(/dikdörtgen bir aday çizin/)).toBeTruthy();
    expect(queryByText(/Tüm hassas bilgiler/)).toBeNull();
  });

  it("lists, navigates, deletes, and clears pending physical-page regions", async () => {
    const state = createSecureRedactionState();
    state.addRegion(region());
    const callbacks = props(state);
    const { getByText, getByLabelText } = render(SecureRedactionPanel, { props: callbacks });
    await fireEvent.click(getByText("Sayfa 3"));
    expect(callbacks.onNavigate).toHaveBeenCalledWith(2);
    await fireEvent.click(getByLabelText(/Adayı sil/));
    expect(callbacks.onRemoveRegion).toHaveBeenCalledWith("region-1");
    await fireEvent.click(getByText("Tüm adayları temizle"));
    expect(callbacks.onClearRegions).toHaveBeenCalledTimes(1);
  });

  it("renders planning progress and a current-job cancellation control", async () => {
    const state = createSecureRedactionState();
    state.beginPlanning("plan-1", "session-1", 1);
    const callbacks = props(state);
    const { getByText } = render(SecureRedactionPanel, { props: callbacks });
    expect(getByText("Plan hazırlanıyor…")).toBeTruthy();
    await fireEvent.click(getByText("Planlamayı iptal et"));
    expect(callbacks.onCancelPlan).toHaveBeenCalledTimes(1);
  });

  it("separates blockers, warnings, losses, and only required acknowledgements", async () => {
    const state = createSecureRedactionState();
    state.addRegion(region());
    state.beginPlanning("plan-1", "session-1", 1);
    state.completePlanning("plan-1", plan({
      executable: false,
      blockers: [{ code: "SIGNED", message: "İmzalı belgeler bu profilde engellenir." }],
    }));
    const callbacks = props(state);
    const { getByText, getAllByRole, queryByText } = render(SecureRedactionPanel, { props: callbacks });
    expect(getByText(/İmzalı belgeler bu profilde engellenir/)).toBeTruthy();
    expect(getByText(/Renk doğruluğunu gözden geçirin/)).toBeTruthy();
    expect(getByText("Vektör kalitesi")).toBeTruthy();
    expect(getByText(/Metin seçimi ve arama/)).toBeTruthy();
    expect(queryByText(/İmza geçersizleşmesi riskini kabul/)).toBeNull();
    const execute = getAllByRole("button").find((button) => button.textContent.includes("Doğrulanmış çıktı"));
    expect(execute.disabled).toBe(true);
  });

  it("enables Save execution only for a latest authoritative ready plan", async () => {
    const state = createSecureRedactionState();
    state.addRegion(region());
    state.beginPlanning("plan-1", "session-1", 1);
    state.completePlanning("plan-1", plan({ readyForExecution: true }));
    const callbacks = props(state);
    const { getByText } = render(SecureRedactionPanel, { props: callbacks });
    const execute = getByText("Doğrulanmış çıktı kaydet");
    expect(execute.disabled).toBe(false);
    await fireEvent.click(execute);
    expect(callbacks.onExecute).toHaveBeenCalledTimes(1);
  });

  it("keeps all backend-required acknowledgement controls visible while the authoritative plan is stale", async () => {
    const state = createSecureRedactionState();
    state.addRegion(region());
    state.beginPlanning("plan-1", "session-1", 1);
    state.completePlanning("plan-1", plan());
    const callbacks = props(state, {
      onAcknowledgementChange: (name, value) => state.setAcknowledgement(name, value),
    });
    const view = render(SecureRedactionPanel, { props: callbacks });
    const checkboxes = view.getAllByRole("checkbox");
    expect(checkboxes).toHaveLength(2);
    await fireEvent.click(checkboxes[0]);
    expect(state.plan).toBeNull();
    expect(state.reviewPlan).not.toBeNull();
    expect(view.getAllByRole("checkbox")).toHaveLength(2);
    expect(view.getByText(/inceleme özeti stale/)).toBeTruthy();
    expect(view.getByText("Doğrulanmış çıktı kaydet").disabled).toBe(true);
  });

  it("renders execution/cancel and committed durability truth", async () => {
    const state = createSecureRedactionState();
    state.beginPlanning("ready", "session-1", 1);
    state.completePlanning("ready", plan({ readyForExecution: true }));
    state.beginExecution("exec-1", {});
    const callbacks = props(state);
    const view = render(SecureRedactionPanel, { props: callbacks });
    expect(view.getByText(/Güvenli redaksiyon uygulanıyor/)).toBeTruthy();
    await fireEvent.click(view.getByText("İptal et"));
    expect(callbacks.onCancelExecution).toHaveBeenCalledTimes(1);

    state.completeExecution("exec-1", {
      published: true,
      publicationReport: { publishedPath: "/tmp/output.pdf", commitState: "new_committed_durability_unconfirmed" },
      verificationReport: { isVerified: true },
      transformReport: { affectedPages: [0, 2], rasterDpi: 300 },
    });
    await fireEvent.click(document.body);
    expect(view.getByText(/durability onayı tamamlanamadı/)).toBeTruthy();
    expect(view.getByText("/tmp/output.pdf")).toBeTruthy();
    expect(view.getByText("Başarılı")).toBeTruthy();
  });

  it("shows recovery metadata without flattening indeterminate disk state", () => {
    const state = createSecureRedactionState();
    state.beginExecution("exec-1", {});
    state.failExecution("exec-1", {
      kind: "recovery_required",
      message: "Dosya sistemi sonucu kurtarma gerektiriyor.",
      commitState: "recovery_required",
      recoveryPath: "/tmp/recovery.pdf",
    });
    const { getByText } = render(SecureRedactionPanel, { props: props(state) });
    expect(getByText("Dosya sistemi sonucu kurtarma gerektiriyor.")).toBeTruthy();
    expect(getByText(/\/tmp\/recovery.pdf/)).toBeTruthy();
    expect(getByText(/recovery_required/)).toBeTruthy();
  });
});
