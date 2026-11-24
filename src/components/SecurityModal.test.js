import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";

const dialogMocks = vi.hoisted(() => ({ save: vi.fn() }));
const sessionMocks = vi.hoisted(() => ({
  getSessionFormInfo: vi.fn(),
  saveSessionSecurityCopy: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => dialogMocks);
vi.mock("../services/backend/session.js", () => sessionMocks);

import SecurityModal from "./SecurityModal.svelte";

// The backend values mirror document_security::tests and the session command test:
// sanitization/signature.pdf → signature_count 1 / signaturesInvalidated true,
// minimal.pdf → 0 / false.
const SIGNED = { info: { form_type: "AcroForm", signature_count: 1 }, report: { signaturesInvalidated: true } };
const UNSIGNED = { info: { form_type: "None", signature_count: 0 }, report: { signaturesInvalidated: false } };
const NOTE = /Bu belge dijital imza içeriyor\. Kopya baştan yazılır/;

describe("SecurityModal signatures", () => {
  const originalAnimate = Element.prototype.animate;

  beforeEach(() => {
    vi.clearAllMocks();
    Element.prototype.animate = vi.fn(() => ({ finished: Promise.resolve(), cancel: vi.fn(), onfinish: null }));
    dialogMocks.save.mockResolvedValue("/çıktı/belge_korumasiz.pdf");
  });

  afterEach(() => {
    cleanup();
    Element.prototype.animate = originalAnimate;
  });

  function renderModal(document) {
    sessionMocks.getSessionFormInfo.mockResolvedValue(document.info);
    sessionMocks.saveSessionSecurityCopy.mockResolvedValue(document.report);
    const onsaved = vi.fn();
    render(SecurityModal, { props: { sessionId: "session-1", contentGeneration: 2, fileName: "belge.pdf", onsaved, onclose: vi.fn() } });
    return { onsaved };
  }

  it.each(["Parola ile koru", "Güvenliği kaldır"])("warns before saving a signed document (%s)", async (mode) => {
    renderModal(SIGNED);
    await fireEvent.click(screen.getByRole("button", { name: mode }));
    expect(await screen.findByText(NOTE)).toBeTruthy();
    expect(sessionMocks.getSessionFormInfo).toHaveBeenCalledWith("session-1", 2);
  });

  it("shows no note for an unsigned document", async () => {
    renderModal(UNSIGNED);
    await waitFor(() => expect(sessionMocks.getSessionFormInfo).toHaveBeenCalled());
    await Promise.resolve();
    expect(screen.queryByText(NOTE)).toBeNull();
  });

  it.each([
    ["signed", SIGNED, "Korumasız kopya kaydedildi. Belgedeki dijital imzalar kopyada geçersizdir."],
    ["unsigned", UNSIGNED, "Korumasız kopya kaydedildi."],
  ])("reports the backend's signature result after saving (%s)", async (_label, document, message) => {
    const { onsaved } = renderModal(document);
    await fireEvent.click(screen.getByRole("button", { name: "Güvenliği kaldır" }));
    await fireEvent.click(screen.getByRole("button", { name: "Kopyayı kaydet…" }));
    await waitFor(() => expect(onsaved).toHaveBeenCalledWith(message));
    expect(sessionMocks.saveSessionSecurityCopy).toHaveBeenCalledWith("session-1", 2, { op: "remove" }, "/çıktı/belge_korumasiz.pdf");
  });
});
