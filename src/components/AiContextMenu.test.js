import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, cleanup, fireEvent } from "@testing-library/svelte";
import AiContextMenu from "./AiContextMenu.svelte";
import { aiState } from "../stores/aiState.svelte.js";
import * as backendAi from "../services/backend/ai.js";

vi.mock("../services/backend/ai.js", () => ({
  summarizeSelectionOrPage: vi.fn(),
  cancelAiRequest: vi.fn(),
}));

const originalAnimate = Element.prototype.animate;

describe("AiContextMenu data-flow disclosure tests", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    Element.prototype.animate = vi.fn(() => ({
      cancel: vi.fn(),
      finished: Promise.resolve(),
    }));
    aiState.llmModel = "llama3.2:latest";
    aiState.embedModel = "nomic-embed-text:latest";
    aiState.activeProfile = {
      policy_version: 1,
      mode: "Loopback",
      scheme: "http",
      canonical_host: "127.0.0.1",
      effective_port: 11434,
      canonical_origin: "http://127.0.0.1:11434",
      endpoint_fingerprint: "fp_test",
      created_at_ms: 1000,
    };

    window.getSelection = vi.fn(() => ({
      toString: () => "Selected sample text for summary",
      getRangeAt: () => ({
        getBoundingClientRect: () => ({
          left: 100,
          bottom: 200,
        }),
      }),
    }));
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    Element.prototype.animate = originalAnimate;
  });

  it("renders selection summary disclosure in Loopback mode", async () => {
    aiState.activeProfile = {
      mode: "Loopback",
      endpoint_fingerprint: "fp_loopback",
    };
    render(AiContextMenu);

    await fireEvent.mouseUp(document);
    await vi.advanceTimersByTimeAsync(150);

    const expected =
      "Yerel AI — Loopback: Seçili metin, özetleme/açıklama talimatı ve LLM model adı (llama3.2:latest) yapılandırılan endpoint'e iletilir.";
    expect(screen.getByText(expected)).toBeTruthy();
  });

  it("renders selection summary disclosure in PrivateNetwork mode", async () => {
    aiState.activeProfile = {
      mode: "PrivateNetwork",
      endpoint_fingerprint: "fp_private",
    };
    render(AiContextMenu);

    await fireEvent.mouseUp(document);
    await vi.advanceTimersByTimeAsync(150);

    const expected =
      "Özel Ağ AI Endpoint: Seçili metin, özetleme/açıklama talimatı ve LLM model adı (llama3.2:latest) yapılandırılan endpoint'e iletilir.";
    expect(screen.getByText(expected)).toBeTruthy();
  });

  it("renders selection summary disclosure in RemotePublic mode", async () => {
    aiState.activeProfile = {
      mode: "RemotePublic",
      endpoint_fingerprint: "fp_remote",
    };
    render(AiContextMenu);

    await fireEvent.mouseUp(document);
    await vi.advanceTimersByTimeAsync(150);

    const expected =
      "Uzak AI Endpoint: Seçili metin, özetleme/açıklama talimatı ve LLM model adı (llama3.2:latest) yapılandırılan endpoint'e iletilir.";
    expect(screen.getByText(expected)).toBeTruthy();
  });
});
