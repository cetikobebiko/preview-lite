import { describe, expect, it } from "vitest";
import { createUiState } from "./uiState.svelte.js";

describe("ui state", () => {
  it("replaces, rather than stacks, blocking modals", () => {
    const state = createUiState();
    state.openModal("ai_assistant");
    state.openModal("ai_settings");
    expect(state.isModalOpen("ai_assistant")).toBe(false);
    expect(state.isModalOpen("ai_settings")).toBe(true);
  });

  it("keeps docked panels independent from the blocking modal", () => {
    const state = createUiState();
    state.openModal("export");
    state.setPanel("prepress", true);
    state.closeModal();
    expect(state.panels.prepress).toBe(true);
  });

  it("manages sensitiveDataScan panel state through isSensitiveDataScanPanelOpen", () => {
    const state = createUiState();
    expect(state.panels.sensitiveDataScan).toBe(false);
    expect(state.isSensitiveDataScanPanelOpen).toBe(false);
    state.isSensitiveDataScanPanelOpen = true;
    expect(state.panels.sensitiveDataScan).toBe(true);
    expect(state.isSensitiveDataScanPanelOpen).toBe(true);
  });

  it("keeps secure-redaction panel state independent from scan and other panels", () => {
    const state = createUiState();
    state.setPanel("sensitiveDataScan", true);
    state.isSecureRedactionPanelOpen = true;
    state.setPanel("prepress", true);
    state.setPanel("sensitiveDataScan", false);
    expect(state.panels.secureRedaction).toBe(true);
    expect(state.panels.prepress).toBe(true);
    state.isSecureRedactionPanelOpen = false;
    expect(state.panels.secureRedaction).toBe(false);
  });
});
