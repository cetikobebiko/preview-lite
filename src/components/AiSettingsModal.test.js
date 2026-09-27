import { describe, it, expect, beforeEach, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import AiSettingsModal from "./AiSettingsModal.svelte";
import { aiState } from "../stores/aiState.svelte.js";
import * as backendAi from "../services/backend/ai.js";

vi.mock("../services/backend/ai.js", () => ({
  previewAiEndpoint: vi.fn(),
  configureAiEndpoint: vi.fn(),
  getAiEndpointStatus: vi.fn(),
}));

const originalAnimate = Element.prototype.animate;

describe("AiSettingsModal component tests", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    Element.prototype.animate = vi.fn(() => ({
      cancel: vi.fn(),
      finished: Promise.resolve(),
    }));
    aiState.endpointDraft = "http://127.0.0.1:11434";
    aiState.remoteConsent = false;
    aiState.privateConsent = false;
    aiState.llmModel = "llama3.2:latest";
    aiState.embedModel = "nomic-embed-text:latest";
    aiState.activeProfile = {
      policy_version: 1,
      mode: "Loopback",
      scheme: "http",
      canonical_host: "127.0.0.1",
      effective_port: 11434,
      canonical_origin: "http://127.0.0.1:11434",
      endpoint_fingerprint: "fp_test_init",
      created_at_ms: 1000,
    };
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    Element.prototype.animate = originalAnimate;
  });

  it("renders with current drafts and displays mode preview", async () => {
    backendAi.previewAiEndpoint.mockResolvedValueOnce(aiState.activeProfile);
    render(AiSettingsModal);

    expect(screen.getByText("Yapay Zeka Ayarları (Ollama)")).toBeTruthy();
    expect(screen.getByDisplayValue("http://127.0.0.1:11434")).toBeTruthy();
    expect(screen.getByDisplayValue("llama3.2:latest")).toBeTruthy();
    expect(screen.getByDisplayValue("nomic-embed-text:latest")).toBeTruthy();
  });

  it("debounces preview requests on user input", async () => {
    backendAi.previewAiEndpoint.mockResolvedValue({
      policy_version: 1,
      mode: "Loopback",
      scheme: "http",
      canonical_host: "127.0.0.1",
      effective_port: 11435,
      canonical_origin: "http://127.0.0.1:11435",
      endpoint_fingerprint: "fp_new",
      created_at_ms: 2000,
    });

    render(AiSettingsModal);
    const input = screen.getByDisplayValue("http://127.0.0.1:11434");

    await fireEvent.input(input, { target: { value: "http://127.0.0.1:11435" } });

    // Not immediately called due to debounce
    expect(backendAi.previewAiEndpoint).not.toHaveBeenCalledWith("http://127.0.0.1:11435");

    // Fast-forward 300ms
    await vi.advanceTimersByTimeAsync(300);

    expect(backendAi.previewAiEndpoint).toHaveBeenCalledWith("http://127.0.0.1:11435");
  });

  it("discards stale preview response when revision has changed", async () => {
    backendAi.previewAiEndpoint.mockResolvedValueOnce(aiState.activeProfile);
    render(AiSettingsModal);
    await vi.advanceTimersByTimeAsync(50);

    let resolveFirst;
    const firstPromise = new Promise((res) => {
      resolveFirst = res;
    });

    backendAi.previewAiEndpoint
      .mockImplementationOnce(() => firstPromise)
      .mockResolvedValueOnce({
        policy_version: 1,
        mode: "RemotePublic",
        scheme: "https",
        canonical_host: "remote.example.com",
        effective_port: 443,
        canonical_origin: "https://remote.example.com",
        endpoint_fingerprint: "fp_remote",
        created_at_ms: 3000,
      });

    const input = screen.getByDisplayValue("http://127.0.0.1:11434");

    // First edit
    await fireEvent.input(input, { target: { value: "http://127.0.0.1:9999" } });
    await vi.advanceTimersByTimeAsync(300);

    // Second edit before first preview resolves
    await fireEvent.input(input, { target: { value: "https://remote.example.com" } });
    await vi.advanceTimersByTimeAsync(300);

    // Now resolve first stale preview with Loopback
    resolveFirst({
      policy_version: 1,
      mode: "Loopback",
      scheme: "http",
      canonical_host: "127.0.0.1",
      effective_port: 9999,
      canonical_origin: "http://127.0.0.1:9999",
      endpoint_fingerprint: "fp_stale",
      created_at_ms: 1000,
    });

    await vi.advanceTimersByTimeAsync(50);

    // The second preview (RemotePublic) should be displayed, not the first stale one
    expect(screen.getByText("https://remote.example.com")).toBeTruthy();
    expect(screen.queryByText("http://127.0.0.1:9999")).toBeNull();
  });

  it("blocks save and shows error banner when endpoint validation fails", async () => {
    backendAi.previewAiEndpoint.mockRejectedValue(new Error("URL length exceeds 2048 characters"));

    render(AiSettingsModal);
    const input = screen.getByDisplayValue("http://127.0.0.1:11434");

    await fireEvent.input(input, { target: { value: "http://bad-url" } });
    await vi.advanceTimersByTimeAsync(300);

    expect(screen.getByText(/Endpoint Hatası:/)).toBeTruthy();
    const saveBtn = screen.getByText("Kaydet");
    expect(saveBtn.hasAttribute("disabled")).toBe(true);
  });

  it("2.1 & 2.3: renders remote and private-network consent checkboxes unchecked by default", async () => {
    backendAi.previewAiEndpoint.mockResolvedValueOnce({
      policy_version: 1,
      mode: "PrivateNetwork",
      scheme: "https",
      canonical_host: "192.168.1.50",
      effective_port: 8443,
      canonical_origin: "https://192.168.1.50:8443",
      endpoint_fingerprint: "fp_priv",
      created_at_ms: 1000,
    });

    render(AiSettingsModal);
    await vi.advanceTimersByTimeAsync(300);

    const remoteCheckbox = screen.getByLabelText(/Uzak AI Endpoint Kullanımına Onay Ver/);
    const privateCheckbox = screen.getByLabelText(/Özel Ağ Endpoint Kullanımına İkinci Onay Ver/);

    expect(remoteCheckbox.checked).toBe(false);
    expect(privateCheckbox.checked).toBe(false);
  });

  it("2.2: resets consent checkboxes when user modifies endpoint URL", async () => {
    backendAi.previewAiEndpoint.mockResolvedValue({
      policy_version: 1,
      mode: "RemotePublic",
      scheme: "https",
      canonical_host: "remote.example.com",
      effective_port: 443,
      canonical_origin: "https://remote.example.com",
      endpoint_fingerprint: "fp_rem",
      created_at_ms: 1000,
    });

    render(AiSettingsModal);
    await vi.advanceTimersByTimeAsync(300);

    const remoteCheckbox = screen.getByLabelText(/Uzak AI Endpoint Kullanımına Onay Ver/);
    const privateCheckbox = screen.getByLabelText(/Özel Ağ Endpoint Kullanımına İkinci Onay Ver/);

    // User checks consent
    await fireEvent.click(remoteCheckbox);
    await fireEvent.click(privateCheckbox);
    expect(remoteCheckbox.checked).toBe(true);
    expect(privateCheckbox.checked).toBe(true);

    // User changes endpoint URL
    const input = screen.getByDisplayValue("http://127.0.0.1:11434");
    await fireEvent.input(input, { target: { value: "https://new-endpoint.example.com" } });

    // Checkboxes are immediately cleared
    expect(remoteCheckbox.checked).toBe(false);
    expect(privateCheckbox.checked).toBe(false);
  });
});
