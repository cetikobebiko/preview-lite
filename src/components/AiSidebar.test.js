import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, cleanup, fireEvent } from "@testing-library/svelte";
import AiSidebar from "./AiSidebar.svelte";
import { aiState } from "../stores/aiState.svelte.js";
import { aiController } from "../services/app/aiController.js";
import * as backendAi from "../services/backend/ai.js";

vi.mock("../services/backend/ai.js", () => ({
  buildSemanticIndex: vi.fn(),
  queryDocumentRag: vi.fn(),
  cancelAiRequest: vi.fn(),
}));

const originalAnimate = Element.prototype.animate;

describe("AiSidebar data-flow disclosure tests", () => {
  beforeEach(() => {
    vi.clearAllMocks();
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
  });

  afterEach(() => {
    cleanup();
    Element.prototype.animate = originalAnimate;
  });

  it("renders indexing disclosure in Loopback mode when messages are empty", () => {
    aiState.activeProfile = {
      mode: "Loopback",
      endpoint_fingerprint: "fp_loopback",
    };
    render(AiSidebar, { sessionId: "s1", contentGeneration: 1 });

    const expected =
      "Yerel AI — Loopback: Belgeden çıkarılan metin parçaları ve embedding model adı (nomic-embed-text:latest) yapılandırılan endpoint'e iletilir.";
    expect(screen.getByText(expected)).toBeTruthy();
  });

  it("renders indexing disclosure in PrivateNetwork mode when messages are empty", () => {
    aiState.activeProfile = {
      mode: "PrivateNetwork",
      endpoint_fingerprint: "fp_private",
    };
    render(AiSidebar, { sessionId: "s1", contentGeneration: 1 });

    const expected =
      "Özel Ağ AI Endpoint: Belgeden çıkarılan metin parçaları ve embedding model adı (nomic-embed-text:latest) yapılandırılan endpoint'e iletilir.";
    expect(screen.getByText(expected)).toBeTruthy();
  });

  it("renders indexing disclosure in RemotePublic mode when messages are empty", () => {
    aiState.activeProfile = {
      mode: "RemotePublic",
      endpoint_fingerprint: "fp_remote",
    };
    render(AiSidebar, { sessionId: "s1", contentGeneration: 1 });

    const expected =
      "Uzak AI Endpoint: Belgeden çıkarılan metin parçaları ve embedding model adı (nomic-embed-text:latest) yapılandırılan endpoint'e iletilir.";
    expect(screen.getByText(expected)).toBeTruthy();
  });

  it("switches to RAG chat disclosure once messages exist", async () => {
    aiState.activeProfile = {
      mode: "Loopback",
      endpoint_fingerprint: "fp_loopback",
    };
    render(AiSidebar, { sessionId: "s1", contentGeneration: 1 });

    const input = screen.getByPlaceholderText("Belge hakkında soru sorun...");
    await fireEvent.input(input, { target: { value: "Test question" } });
    await fireEvent.keyDown(input, { key: "Enter" });

    const expected =
      "Yerel AI — Loopback: Kullanıcı sorusu, getirilen belge bağlam parçaları, sınırlandırılmış sohbet geçmişi ve LLM model adı (llama3.2:latest) yapılandırılan endpoint'e iletilir; semantik erişim sırasında embedding modeli (nomic-embed-text:latest) ile iletişim kurulabilir.";
    expect(screen.getByText(expected)).toBeTruthy();
  });

  it("2.4: Cancel button triggers cancellation on active stream handle and renders cancelled label", async () => {
    aiState.activeProfile = {
      mode: "Loopback",
      endpoint_fingerprint: "fp_loopback",
    };
    aiState.setIndexedMetadata({
      sessionId: "s1",
      generation: 1,
      embedModel: "nomic-embed-text:latest",
      endpointFingerprint: "fp_loopback",
      chunkSize: 500,
      chunkOverlap: 50,
    });

    let cancelPromiseResolve;
    const cancelCalled = new Promise((resolve) => {
      cancelPromiseResolve = resolve;
    });

    let doneReject;
    const mockDone = new Promise((_, reject) => {
      doneReject = reject;
    });

    vi.spyOn(aiController, "startQueryDocumentRag").mockImplementation(() => {
      aiState.isStreaming = true;
      return {
        requestId: "req_test_cancel_ui",
        done: mockDone,
        cancel: vi.fn().mockImplementation(async () => {
          const err = new Error("AI request cancelled");
          err.code = "AI_REQUEST_CANCELLED";
          doneReject(err);
          cancelPromiseResolve();
          aiState.isStreaming = false;
        }),
      };
    });

    render(AiSidebar, { sessionId: "s1", contentGeneration: 1 });

    const input = screen.getByPlaceholderText("Belge hakkında soru sorun...");
    await fireEvent.input(input, { target: { value: "Test question to cancel" } });
    await fireEvent.keyDown(input, { key: "Enter" });

    // Stream is active, cancel button is rendered
    const cancelBtn = screen.getByText("İptal Et");
    expect(cancelBtn).toBeTruthy();

    await fireEvent.click(cancelBtn);
    await cancelCalled;

    expect(screen.getByText(/\[İptal Edildi\]/)).toBeTruthy();
  });

  it("2.5: Failed document indexing does NOT present false success in chat messages", async () => {
    const alertMock = vi.spyOn(window, "alert").mockImplementation(() => {});
    backendAi.buildSemanticIndex.mockRejectedValueOnce(new Error("Ollama unavailable"));

    render(AiSidebar, { sessionId: "s1", contentGeneration: 1 });

    const indexBtn = screen.getByText("Endpoint’e Göndererek İndeksle");
    await fireEvent.click(indexBtn);

    // Alert was triggered with error
    expect(alertMock).toHaveBeenCalled();
    expect(alertMock.mock.calls[0][0]).toContain("İndeksleme başarısız oldu");

    // Success message is NOT shown
    expect(
      screen.queryByText(/Belge başarıyla okundu ve yapay zeka hafızasına eklendi/),
    ).toBeNull();
  });
});
