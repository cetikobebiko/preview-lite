import { describe, it, expect, beforeEach, vi } from "vitest";
import { callBackend } from "../backend/client.js";
import { aiController } from "./aiController.js";
import { aiState } from "../../stores/aiState.svelte.js";

vi.mock("../backend/client.js", () => ({
  callBackend: vi.fn(),
  callBackendProfiled: vi.fn(),
}));

describe("aiController unit and integration tests", () => {
  let storage = {};
  const mockStorage = {
    getItem: (key) => storage[key] ?? null,
    setItem: (key, val) => {
      storage[key] = String(val);
    },
    removeItem: (key) => {
      delete storage[key];
    },
    clear: () => {
      storage = {};
    },
  };

  beforeEach(() => {
    vi.clearAllMocks();
    callBackend.mockResolvedValue({});
    storage = {};
    globalThis.localStorage = mockStorage;
    aiState.error = null;
    aiState.isStreaming = false;
    aiState.summaryStreamingText = "";
    aiState.clearIndexedMetadata();
    aiState.llmModel = "llama3.2:latest";
    aiState.embedModel = "nomic-embed-text:latest";
    aiState.chunkSize = 500;
    aiState.chunkOverlap = 50;
    aiState.activeProfile = {
      policy_version: 1,
      mode: "Loopback",
      scheme: "http",
      canonical_host: "127.0.0.1",
      effective_port: 11434,
      canonical_origin: "http://127.0.0.1:11434",
      endpoint_fingerprint: "fp_active_test",
      created_at_ms: 1000,
    };
    aiController.init();
  });

  it("migrates legacy localStorage keys on initialization", () => {
    expect(aiState.endpointDraft).toBe("http://127.0.0.1:11434");
  });

  it("handles valid stream event sequence", async () => {
    const handle = aiController.startSummary("test text", "short");
    handle.done.catch(() => {});
    const reqId = handle.requestId;

    aiController.handleStreamEvent({
      schemaVersion: 1,
      requestId: reqId,
      sequence: 1,
      kind: "started",
      endpointFingerprint: aiState.activeProfile.endpoint_fingerprint,
      terminal: false,
    });
    expect(aiState.error).toBeNull();

    aiController.handleStreamEvent({
      schemaVersion: 1,
      requestId: reqId,
      sequence: 2,
      kind: "delta",
      delta: "Hello",
      endpointFingerprint: aiState.activeProfile.endpoint_fingerprint,
      terminal: false,
    });
    expect(aiState.error).toBeNull();
    expect(aiState.summaryStreamingText).toBe("Hello");
  });

  it("detects sequence regression and sets AI_STREAM_SEQUENCE_ERROR", () => {
    const handle = aiController.startSummary("test text", "short");
    handle.done.catch(() => {});
    const reqId = handle.requestId;

    aiController.handleStreamEvent({
      schemaVersion: 1,
      requestId: reqId,
      sequence: 5,
      kind: "delta",
      delta: "First",
      endpointFingerprint: aiState.activeProfile.endpoint_fingerprint,
      terminal: false,
    });

    // Sequence regression (3 <= 5)
    aiController.handleStreamEvent({
      schemaVersion: 1,
      requestId: reqId,
      sequence: 3,
      kind: "delta",
      delta: "Regression",
      endpointFingerprint: aiState.activeProfile.endpoint_fingerprint,
      terminal: false,
    });

    expect(aiState.error).not.toBeNull();
    expect(aiState.error.code).toBe("AI_STREAM_SEQUENCE_ERROR");
  });

  it("configures endpoint via backend and updates store", async () => {
    const mockProfile = {
      policy_version: 1,
      mode: "Loopback",
      scheme: "http",
      canonical_host: "127.0.0.1",
      effective_port: 11434,
      canonical_origin: "http://127.0.0.1:11434",
      endpoint_fingerprint: "fp123",
      created_at_ms: 1000,
    };
    callBackend.mockImplementation(async (cmd) => {
      if (cmd === "configureAiEndpoint") return mockProfile;
      return {};
    });

    const res = await aiController.configureEndpoint("http://127.0.0.1:11434");
    expect(res).toEqual(mockProfile);
    expect(aiState.activeProfile).toEqual(mockProfile);
  });

  it("hydrates endpoint status from backend on startup", async () => {
    const mockProfile = {
      policy_version: 1,
      mode: "Loopback",
      scheme: "http",
      canonical_host: "127.0.0.1",
      effective_port: 11434,
      canonical_origin: "http://127.0.0.1:11434",
      endpoint_fingerprint: "hydrated_fp",
      created_at_ms: 2000,
    };
    callBackend.mockImplementation(async (cmd) => {
      if (cmd === "getAiEndpointStatus") return mockProfile;
      return {};
    });

    const res = await aiController.fetchEndpointStatus();
    expect(res).toEqual(mockProfile);
    expect(aiState.activeProfile.endpoint_fingerprint).toBe("hydrated_fp");
    expect(aiState.endpointDraft).toBe("http://127.0.0.1:11434");
  });

  it("ensureHydrated auto-fetches endpoint when unhydrated", async () => {
    aiState.activeProfile = null;
    const mockProfile = {
      policy_version: 1,
      mode: "Loopback",
      scheme: "http",
      canonical_host: "127.0.0.1",
      effective_port: 11434,
      canonical_origin: "http://127.0.0.1:11434",
      endpoint_fingerprint: "auto_hydrated_fp",
      created_at_ms: 2000,
    };
    callBackend.mockImplementation(async (cmd) => {
      if (cmd === "getAiEndpointStatus") return mockProfile;
      return {};
    });

    const res = await aiController.ensureHydrated();
    expect(res).toEqual(mockProfile);
    expect(aiState.isHydrated).toBe(true);
  });

  it("propagates cancellation error when backend cancel rejects", async () => {
    const errObj = { code: "AI_REQUEST_NOT_CANCELLABLE", message: "Not cancellable" };
    callBackend.mockImplementation(async (cmd) => {
      if (cmd === "cancelAiRequest") throw errObj;
      return {};
    });

    await expect(aiController.cancelRequest("req_uncancellable")).rejects.toEqual(errObj);
    expect(aiState.error).toEqual(errObj);
  });

  it("discards stream events when guard is no longer current", () => {
    let currentSessionId = "session_A";
    let currentGen = 1;

    aiController.init({
      getGuard: () => ({ sessionId: currentSessionId, contentGeneration: currentGen }),
      isGuardCurrent: (guard) =>
        guard && guard.sessionId === currentSessionId && guard.contentGeneration === currentGen,
    });

    const handle = aiController.startQueryDocumentRag("session_A", 1, "Question");
    handle.done.catch(() => {});
    const reqId = handle.requestId;

    // Session transitions to session_B
    currentSessionId = "session_B";

    aiController.handleStreamEvent({
      schemaVersion: 1,
      requestId: reqId,
      sequence: 1,
      kind: "delta",
      delta: "Stale answer segment",
      endpointFingerprint: aiState.activeProfile.endpoint_fingerprint,
      terminal: false,
    });

    expect(aiState.error).toBeNull();
  });

  // Gap D: Real sessionJob unwrapping to null and index readiness
  it("Gap D: real sessionJob unwraps completed value: null and aiController marks index ready", async () => {
    callBackend.mockImplementation(async (cmd) => {
      if (cmd === "buildSemanticIndex") {
        return {
          status: "completed",
          jobId: "00000000-0000-4000-8000-000000000001",
          sessionId: "s1",
          contentGeneration: 1,
          value: null,
          cleanup: { status: "cleaned" },
        };
      }
      return {};
    });

    expect(aiState.isIndexReadyFor("s1", 1)).toBe(false);

    // Call real backendAi through aiController
    const res = await aiController.buildSemanticIndex("s1", 1);
    expect(res).toBeNull();
    expect(aiState.isIndexReadyFor("s1", 1)).toBe(true);
    expect(aiState.isIndexReadyFor("s1", 2)).toBe(false);
    expect(aiState.isIndexReadyFor("other_session", 1)).toBe(false);
  });

  it("R1-R3: isIndexReadyFor fails closed when profile/fingerprint is absent or mismatched", () => {
    aiState.setIndexedMetadata({
      sessionId: "s1",
      generation: 1,
      embedModel: "nomic-embed-text:latest",
      endpointFingerprint: "fp_orig",
      chunkSize: 500,
      chunkOverlap: 50,
    });
    aiState.embedModel = "nomic-embed-text:latest";
    aiState.chunkSize = 500;
    aiState.chunkOverlap = 50;

    // Matching profile
    aiState.activeProfile = { endpoint_fingerprint: "fp_orig", mode: "Loopback" };
    expect(aiState.isIndexReadyFor("s1", 1)).toBe(true);

    // R1: Missing activeProfile fails closed
    aiState.activeProfile = null;
    expect(aiState.isIndexReadyFor("s1", 1)).toBe(false);

    // R2: Mismatched fingerprint fails closed
    aiState.activeProfile = { endpoint_fingerprint: "fp_different", mode: "Loopback" };
    expect(aiState.isIndexReadyFor("s1", 1)).toBe(false);

    // R3: Mismatched chunk policy fails closed
    aiState.activeProfile = { endpoint_fingerprint: "fp_orig", mode: "Loopback" };
    aiState.chunkSize = 1000;
    expect(aiState.isIndexReadyFor("s1", 1)).toBe(false);
  });

  // Gap I: Explicit P1–P11 Tests for persisted settings
  describe("P1–P11: Persisted settings individual requirement tests", () => {
    it("P1: Valid schema v1 loads all expected fields (endpointDraft, llmModel, embedModel)", () => {
      localStorage.setItem(
        "ai_settings_v1",
        JSON.stringify({
          schemaVersion: 1,
          endpointDraft: "http://127.0.0.1:11435",
          llmModel: "custom-llm:latest",
          embedModel: "custom-embed:latest",
        }),
      );
      aiController._initStorageMigration();
      expect(aiState.endpointDraft).toBe("http://127.0.0.1:11435");
      expect(aiState.llmModel).toBe("custom-llm:latest");
      expect(aiState.embedModel).toBe("custom-embed:latest");
    });

    it("P2: Unknown schema version (schemaVersion: 2 or 0) safely ignored", () => {
      localStorage.setItem(
        "ai_settings_v1",
        JSON.stringify({ schemaVersion: 2, endpointDraft: "http://127.0.0.1:9999" }),
      );
      aiState.endpointDraft = "http://127.0.0.1:11434";
      aiController._initStorageMigration();
      expect(aiState.endpointDraft).toBe("http://127.0.0.1:11434");
    });

    it("P3: Malformed JSON syntax in localStorage fails safely", () => {
      localStorage.setItem("ai_settings_v1", "{ not valid json syntax");
      aiState.endpointDraft = "http://127.0.0.1:11434";
      aiController._initStorageMigration();
      expect(aiState.endpointDraft).toBe("http://127.0.0.1:11434");
    });

    it("P4: Partial valid fields loaded safely while missing sibling fields retain defaults", () => {
      localStorage.setItem(
        "ai_settings_v1",
        JSON.stringify({
          schemaVersion: 1,
          llmModel: "my-model:7b",
        }),
      );
      aiState.endpointDraft = "http://127.0.0.1:11434";
      aiState.embedModel = "nomic-embed-text:latest";
      aiController._initStorageMigration();
      expect(aiState.llmModel).toBe("my-model:7b");
      expect(aiState.endpointDraft).toBe("http://127.0.0.1:11434");
      expect(aiState.embedModel).toBe("nomic-embed-text:latest");
    });

    it("P5: Wrong field types (endpointDraft: 123, llmModel: true) ignored", () => {
      localStorage.setItem(
        "ai_settings_v1",
        JSON.stringify({
          schemaVersion: 1,
          endpointDraft: 123,
          llmModel: true,
          embedModel: { bad: "type" },
        }),
      );
      aiState.endpointDraft = "http://127.0.0.1:11434";
      aiState.llmModel = "llama3.2:latest";
      aiState.embedModel = "nomic-embed-text:latest";
      aiController._initStorageMigration();
      expect(aiState.endpointDraft).toBe("http://127.0.0.1:11434");
      expect(aiState.llmModel).toBe("llama3.2:latest");
      expect(aiState.embedModel).toBe("nomic-embed-text:latest");
    });

    it("P6: Invalid protocol/endpoint draft (ftp://, file:///, ws://) rejected", () => {
      for (const badScheme of [
        "ftp://127.0.0.1:11434",
        "file:///tmp/ai",
        "ws://127.0.0.1:11434",
      ]) {
        localStorage.setItem(
          "ai_settings_v1",
          JSON.stringify({ schemaVersion: 1, endpointDraft: badScheme }),
        );
        aiState.endpointDraft = "http://127.0.0.1:11434";
        aiController._initStorageMigration();
        expect(aiState.endpointDraft).toBe("http://127.0.0.1:11434");
      }
    });

    it("P7: Oversized URL (> 2048 chars) rejected", () => {
      localStorage.setItem(
        "ai_settings_v1",
        JSON.stringify({
          schemaVersion: 1,
          endpointDraft: "http://127.0.0.1:11434/" + "a".repeat(2048),
        }),
      );
      aiState.endpointDraft = "http://127.0.0.1:11434";
      aiController._initStorageMigration();
      expect(aiState.endpointDraft).toBe("http://127.0.0.1:11434");
    });

    it("P8: Invalid model preference (spaces, special characters, length > 128) rejected", () => {
      localStorage.setItem(
        "ai_settings_v1",
        JSON.stringify({
          schemaVersion: 1,
          llmModel: "bad model with spaces and $$$",
          embedModel: "a".repeat(129),
        }),
      );
      aiState.llmModel = "llama3.2:latest";
      aiState.embedModel = "nomic-embed-text:latest";
      aiController._initStorageMigration();
      expect(aiState.llmModel).toBe("llama3.2:latest");
      expect(aiState.embedModel).toBe("nomic-embed-text:latest");
    });

    it("P9: Persisted endpoint remains draft-only until fresh backend configuration", () => {
      localStorage.setItem(
        "ai_settings_v1",
        JSON.stringify({
          schemaVersion: 1,
          endpointDraft: "https://remote.example.com",
        }),
      );
      aiState.activeProfile = null;
      aiController._initStorageMigration();
      expect(aiState.endpointDraft).toBe("https://remote.example.com");
      // activeProfile remains null until backend configure/hydrate
      expect(aiState.activeProfile).toBeNull();
      expect(aiState.isHydrated).toBe(false);
    });

    it("P10: Endpoint change clears frontend index readiness appropriately", async () => {
      aiState.embedModel = "nomic-embed-text:latest";
      aiState.chunkSize = 500;
      aiState.chunkOverlap = 50;
      aiState.setIndexedMetadata({
        sessionId: "s1",
        generation: 1,
        embedModel: "nomic-embed-text:latest",
        endpointFingerprint: "fp_old",
        chunkSize: 500,
        chunkOverlap: 50,
      });
      aiState.activeProfile = { endpoint_fingerprint: "fp_old", mode: "Loopback" };
      expect(aiState.isIndexReadyFor("s1", 1)).toBe(true);

      const newProfile = {
        policy_version: 1,
        mode: "Loopback",
        scheme: "http",
        canonical_host: "127.0.0.1",
        effective_port: 11435,
        canonical_origin: "http://127.0.0.1:11435",
        endpoint_fingerprint: "fp_new",
        created_at_ms: 2000,
      };
      callBackend.mockImplementation(async (cmd) => {
        if (cmd === "configureAiEndpoint") return newProfile;
        return {};
      });

      await aiController.configureEndpoint("http://127.0.0.1:11435");
      expect(aiState.isIndexReadyFor("s1", 1)).toBe(false);
    });

    it("P11: Consent fields in localStorage cannot authorize networking", () => {
      localStorage.setItem(
        "ai_settings_v1",
        JSON.stringify({
          schemaVersion: 1,
          remoteConsent: true,
          privateConsent: true,
        }),
      );
      aiState.remoteConsent = false;
      aiState.privateConsent = false;
      aiController._initStorageMigration();
      expect(aiState.remoteConsent).toBe(false);
      expect(aiState.privateConsent).toBe(false);
    });
  });

  // CLOSURE-1 Acceptance Tests
  describe("CLOSURE-1: Frontend Acceptance Matrix Tests", () => {
    it("2.1: Consent initial states and authorization requirements", () => {
      // Remote and Private consent defaults to false
      aiState.remoteConsent = false;
      aiState.privateConsent = false;

      // Loopback is authorized by default
      aiState.activeProfile = { mode: "Loopback", endpoint_fingerprint: "fp_loop" };
      expect(aiState.isAuthorized).toBe(true);

      // RemotePublic requires remoteConsent
      aiState.activeProfile = { mode: "RemotePublic", endpoint_fingerprint: "fp_rem" };
      expect(aiState.isAuthorized).toBe(false);
      aiState.remoteConsent = true;
      expect(aiState.isAuthorized).toBe(true);

      // PrivateNetwork requires BOTH remoteConsent AND privateConsent
      aiState.activeProfile = { mode: "PrivateNetwork", endpoint_fingerprint: "fp_priv" };
      aiState.remoteConsent = true;
      aiState.privateConsent = false;
      expect(aiState.isAuthorized).toBe(false);

      aiState.remoteConsent = false;
      aiState.privateConsent = true;
      expect(aiState.isAuthorized).toBe(false);

      aiState.remoteConsent = true;
      aiState.privateConsent = true;
      expect(aiState.isAuthorized).toBe(true);
    });

    it("2.2: Endpoint change invalidates previous consent and clears index readiness", async () => {
      // Setup initial RemotePublic endpoint with consent
      aiState.activeProfile = { mode: "RemotePublic", endpoint_fingerprint: "fp_ep1" };
      aiState.remoteConsent = true;
      aiState.privateConsent = false;
      expect(aiState.isAuthorized).toBe(true);

      aiState.setIndexedMetadata({
        sessionId: "s1",
        generation: 1,
        embedModel: "nomic-embed-text:latest",
        endpointFingerprint: "fp_ep1",
        chunkSize: 500,
        chunkOverlap: 50,
      });
      expect(aiState.isIndexReadyFor("s1", 1)).toBe(true);

      // Change endpoint to ep2 without passing consent
      const ep2Profile = {
        policy_version: 1,
        mode: "RemotePublic",
        scheme: "https",
        canonical_host: "ep2.example.com",
        effective_port: 443,
        canonical_origin: "https://ep2.example.com",
        endpoint_fingerprint: "fp_ep2",
        created_at_ms: 2000,
      };
      callBackend.mockResolvedValueOnce(ep2Profile);

      await aiController.configureEndpoint("https://ep2.example.com", false, false);

      expect(aiState.activeProfile.endpoint_fingerprint).toBe("fp_ep2");
      expect(aiState.remoteConsent).toBe(false);
      expect(aiState.privateConsent).toBe(false);
      expect(aiState.isAuthorized).toBe(false);
      expect(aiState.isIndexReadyFor("s1", 1)).toBe(false);
    });

    it("2.4: Cancellation button ownership, request isolation, and terminal safety", async () => {
      const handleA = aiController.startSummary("text A", "short");
      handleA.done.catch(() => {});
      const reqIdA = handleA.requestId;

      const handleB = aiController.startSummary("text B", "short");
      handleB.done.catch(() => {});
      const reqIdB = handleB.requestId;

      expect(reqIdA).not.toBe(reqIdB);

      // Calling cancel on Request A only affects Request A
      callBackend.mockResolvedValue({});
      await handleA.cancel();

      expect(callBackend).toHaveBeenCalledWith("cancelAiRequest", expect.objectContaining({
        requestId: reqIdA,
      }));

      // Stream event for B can still proceed without error
      aiController.handleStreamEvent({
        schemaVersion: 1,
        requestId: reqIdB,
        sequence: 1,
        kind: "delta",
        delta: "Delta for B",
        endpointFingerprint: aiState.activeProfile.endpoint_fingerprint,
        terminal: false,
      });

      expect(aiState.summaryStreamingText).toContain("Delta for B");

      // Repeated cancel on terminal handleA does not throw or corrupt state
      await expect(handleA.cancel()).resolves.toBeUndefined();
    });

    it("2.5: Failed index operation never presents false success", async () => {
      callBackend.mockRejectedValueOnce(new Error("Ollama connection refused"));

      await expect(aiController.buildSemanticIndex("s1", 1)).rejects.toThrow("Ollama connection refused");

      expect(aiState.isIndexing).toBe(false);
      expect(aiState.isIndexReadyFor("s1", 1)).toBe(false);
      expect(aiState.indexedMetadata).toBeNull();
      expect(aiState.error).not.toBeNull();
    });

    it("2.5b: Cancelled index operation rejects with AI_REQUEST_CANCELLED and leaves no ready index", async () => {
      // Prior index
      aiState.setIndexedMetadata({
        sessionId: "s1",
        generation: 1,
        embedModel: "nomic-embed-text:latest",
        endpointFingerprint: "fp_active_test",
        chunkSize: 500,
        chunkOverlap: 50,
      });
      expect(aiState.isIndexReadyFor("s1", 1)).toBe(true);

      const cancelErr = { code: "AI_REQUEST_CANCELLED", message: "Yapay zeka isteği kullanıcı tarafından iptal edildi." };
      callBackend.mockRejectedValueOnce(cancelErr);

      await expect(aiController.buildSemanticIndex("s1", 2)).rejects.toEqual(cancelErr);

      expect(aiState.isIndexing).toBe(false);
      expect(aiState.isIndexReadyFor("s1", 1)).toBe(false);
      expect(aiState.isIndexReadyFor("s1", 2)).toBe(false);
      expect(aiState.indexedMetadata).toBeNull();
      expect(aiState.error).toEqual(cancelErr);
    });

    it("2.5c: Failed rebuild does not present false success over previous valid index", async () => {
      // Initial success for session s1 generation 1
      aiState.setIndexedMetadata({
        sessionId: "s1",
        generation: 1,
        embedModel: "nomic-embed-text:latest",
        endpointFingerprint: "fp_active_test",
        chunkSize: 500,
        chunkOverlap: 50,
      });
      expect(aiState.isIndexReadyFor("s1", 1)).toBe(true);

      // Subsequent build for generation 2 fails
      callBackend.mockRejectedValueOnce(new Error("Embedding dimension mismatch"));

      await expect(aiController.buildSemanticIndex("s1", 2)).rejects.toThrow("Embedding dimension mismatch");

      // After failed build, metadata is cleared fail-closed
      expect(aiState.isIndexReadyFor("s1", 2)).toBe(false);
      expect(aiState.isIndexReadyFor("s1", 1)).toBe(false);
      expect(aiState.indexedMetadata).toBeNull();
    });
  });

  describe("CLOSURE-3: Fail-closed Consent Guard on all 4 AI Operations & Sync Handle API Contract", () => {
    beforeEach(() => {
      callBackend.mockClear();
    });

    it("buildSemanticIndex rejects without backend call on missing/stale consent (RemotePublic & PrivateNetwork)", async () => {
      // RemotePublic without consent
      aiState.activeProfile = {
        mode: "RemotePublic",
        canonical_origin: "https://remote.example.com",
        endpoint_fingerprint: "fp_rem",
      };
      aiState.remoteConsent = false;
      aiState.privateConsent = false;

      await expect(aiController.buildSemanticIndex("s1", 1)).rejects.toMatchObject({
        code: "AI_ENDPOINT_REMOTE_CONSENT_REQUIRED",
      });
      expect(callBackend).not.toHaveBeenCalled();
      expect(aiState.isIndexing).toBe(false);

      // PrivateNetwork with only remote consent
      aiState.activeProfile = {
        mode: "PrivateNetwork",
        canonical_origin: "https://192.168.1.100:8443",
        endpoint_fingerprint: "fp_priv",
      };
      aiState.remoteConsent = true;
      aiState.privateConsent = false;

      await expect(aiController.buildSemanticIndex("s1", 1)).rejects.toMatchObject({
        code: "AI_ENDPOINT_PRIVATE_NETWORK_CONSENT_REQUIRED",
      });
      expect(callBackend).not.toHaveBeenCalled();
      expect(aiState.isIndexing).toBe(false);
    });

    it("buildSemanticIndex succeeds when properly authorized (RemotePublic & PrivateNetwork)", async () => {
      aiState.activeProfile = {
        mode: "RemotePublic",
        canonical_origin: "https://remote.example.com",
        endpoint_fingerprint: "fp_rem",
      };
      aiState.remoteConsent = true;
      aiState.privateConsent = false;

      callBackend.mockResolvedValueOnce({
        status: "completed",
        jobId: "00000000-0000-4000-8000-000000000002",
        sessionId: "s1",
        contentGeneration: 1,
        value: null,
      });

      const res = await aiController.buildSemanticIndex("s1", 1);
      expect(res).toBeNull();
      expect(callBackend).toHaveBeenCalledWith("buildSemanticIndex", expect.anything());
      expect(aiState.isIndexReadyFor("s1", 1)).toBe(true);
    });

    it("semanticSearch rejects without backend call on missing/stale consent (RemotePublic & PrivateNetwork)", async () => {
      // RemotePublic without consent
      aiState.activeProfile = {
        mode: "RemotePublic",
        canonical_origin: "https://remote.example.com",
        endpoint_fingerprint: "fp_rem",
      };
      aiState.remoteConsent = false;

      await expect(aiController.semanticSearch("s1", 1, "test query")).rejects.toMatchObject({
        code: "AI_ENDPOINT_REMOTE_CONSENT_REQUIRED",
      });
      expect(callBackend).not.toHaveBeenCalled();

      // PrivateNetwork without private consent
      aiState.activeProfile = {
        mode: "PrivateNetwork",
        canonical_origin: "https://192.168.1.100:8443",
        endpoint_fingerprint: "fp_priv",
      };
      aiState.remoteConsent = true;
      aiState.privateConsent = false;

      await expect(aiController.semanticSearch("s1", 1, "test query")).rejects.toMatchObject({
        code: "AI_ENDPOINT_PRIVATE_NETWORK_CONSENT_REQUIRED",
      });
      expect(callBackend).not.toHaveBeenCalled();
    });

    it("semanticSearch succeeds when properly authorized (RemotePublic & PrivateNetwork)", async () => {
      aiState.activeProfile = {
        mode: "PrivateNetwork",
        canonical_origin: "https://192.168.1.100:8443",
        endpoint_fingerprint: "fp_priv",
      };
      aiState.remoteConsent = true;
      aiState.privateConsent = true;

      callBackend.mockResolvedValueOnce({ results: [] });

      const res = await aiController.semanticSearch("s1", 1, "test query");
      expect(res).toEqual({ results: [] });
      expect(callBackend).toHaveBeenCalledWith("semanticSearch", expect.anything());
    });

    it("startQueryDocumentRag returns sync handle and rejects handle.done without backend call on missing consent", async () => {
      aiState.activeProfile = {
        mode: "RemotePublic",
        canonical_origin: "https://remote.example.com",
        endpoint_fingerprint: "fp_rem",
      };
      aiState.remoteConsent = false;

      const handle = aiController.startQueryDocumentRag("s1", 1, "Explain this");
      expect(handle).toBeDefined();
      expect(typeof handle.requestId).toBe("string");
      expect(typeof handle.cancel).toBe("function");
      expect(typeof handle.subscribe).toBe("function");
      expect(handle.done).toBeInstanceOf(Promise);

      await expect(handle.done).rejects.toMatchObject({
        code: "AI_ENDPOINT_REMOTE_CONSENT_REQUIRED",
      });

      expect(callBackend).not.toHaveBeenCalled();
      expect(aiState.activeStreamingCount).toBe(0);
      expect(aiState.error).toMatchObject({
        code: "AI_ENDPOINT_REMOTE_CONSENT_REQUIRED",
      });
    });

    it("startSummary returns sync handle and rejects handle.done without backend call on missing consent", async () => {
      aiState.activeProfile = {
        mode: "PrivateNetwork",
        canonical_origin: "https://192.168.1.100:8443",
        endpoint_fingerprint: "fp_priv",
      };
      aiState.remoteConsent = true;
      aiState.privateConsent = false;

      const handle = aiController.startSummary("Sample text to summarize", "short");
      expect(handle).toBeDefined();
      expect(typeof handle.requestId).toBe("string");
      expect(typeof handle.cancel).toBe("function");
      expect(typeof handle.subscribe).toBe("function");
      expect(handle.done).toBeInstanceOf(Promise);

      await expect(handle.done).rejects.toMatchObject({
        code: "AI_ENDPOINT_PRIVATE_NETWORK_CONSENT_REQUIRED",
      });

      expect(callBackend).not.toHaveBeenCalled();
      expect(aiState.activeStreamingCount).toBe(0);
      expect(aiState.error).toMatchObject({
        code: "AI_ENDPOINT_PRIVATE_NETWORK_CONSENT_REQUIRED",
      });
    });

    it("Loopback endpoint allows all 4 AI operations without requiring explicit consent", async () => {
      aiState.activeProfile = {
        mode: "Loopback",
        canonical_origin: "http://127.0.0.1:11434",
        endpoint_fingerprint: "fp_loop",
      };
      aiState.remoteConsent = false;
      aiState.privateConsent = false;

      // 1. buildSemanticIndex
      callBackend.mockResolvedValueOnce({
        status: "completed",
        jobId: "00000000-0000-4000-8000-000000000003",
        sessionId: "s1",
        contentGeneration: 1,
        value: null,
      });
      await expect(aiController.buildSemanticIndex("s1", 1)).resolves.toBeNull();
      expect(callBackend).toHaveBeenCalledWith("buildSemanticIndex", expect.anything());

      // 2. semanticSearch
      callBackend.mockResolvedValueOnce({ results: [] });
      await expect(aiController.semanticSearch("s1", 1, "query")).resolves.toEqual({ results: [] });
      expect(callBackend).toHaveBeenCalledWith("semanticSearch", expect.anything());

      // 3. startQueryDocumentRag
      callBackend.mockResolvedValueOnce({});
      const ragHandle = aiController.startQueryDocumentRag("s1", 1, "question");
      ragHandle.done.catch(() => {});
      expect(ragHandle).toBeDefined();

      // 4. startSummary
      callBackend.mockResolvedValueOnce({});
      const summaryHandle = aiController.startSummary("text", "short");
      summaryHandle.done.catch(() => {});
      expect(summaryHandle).toBeDefined();
    });

    it("AI_ENDPOINT_CONNECT_FAILED is preserved and not masked as generic timeout", async () => {
      const connectFailedErr = {
        code: "AI_ENDPOINT_CONNECT_FAILED",
        message: "Endpoint sunucusuna bağlantı kurulamadı: connection refused",
      };
      callBackend.mockRejectedValueOnce(connectFailedErr);

      await expect(aiController.semanticSearch("s1", 1, "test")).rejects.toMatchObject({
        code: "AI_ENDPOINT_CONNECT_FAILED",
        message: expect.stringContaining("bağlantı kurulamadı"),
      });

      expect(aiState.error).toMatchObject({
        code: "AI_ENDPOINT_CONNECT_FAILED",
      });
    });
  });

  describe("CLOSURE-1: Pre-Hydration & Lifecycle Race Regression Suite", () => {
    it("Test A: startQueryDocumentRag immediate cancellation before hydration", async () => {
      aiState.activeProfile = null;
      let resolveHydration;
      const hydrationPromise = new Promise((resolve) => {
        resolveHydration = resolve;
      });
      callBackend.mockImplementation(async (cmd) => {
        if (cmd === "getAiEndpointStatus") return hydrationPromise;
        return {};
      });

      const handle = aiController.startQueryDocumentRag("s1", 1, "Question");
      expect(handle).toBeDefined();
      expect(aiController._pendingBootstraps.has(handle.requestId)).toBe(true);
      expect(aiController._requests.has(handle.requestId)).toBe(false);

      // Caller cancels immediately before hydration resolves
      await handle.cancel();

      expect(aiController._pendingBootstraps.has(handle.requestId)).toBe(false);
      expect(aiController._requests.has(handle.requestId)).toBe(false);

      // Now hydration resolves
      const profile = {
        policy_version: 1,
        mode: "Loopback",
        canonical_origin: "http://127.0.0.1:11434",
        endpoint_fingerprint: "fp_hydrated_test",
      };
      resolveHydration({ profile });
      await vi.waitFor(() => aiState.isHydrated);

      // Assertions:
      // 1. Backend RAG call count = 0
      expect(callBackend).not.toHaveBeenCalledWith("queryDocumentRag", expect.anything());
      // 2. Backend cancel call count = 0 (since not yet active)
      expect(callBackend).not.toHaveBeenCalledWith("cancelAiRequest", expect.anything());
      // 3. Request registry leak = 0
      expect(aiController._requests.size).toBe(0);
      expect(aiController._pendingBootstraps.size).toBe(0);
      // 4. Active streaming count = 0
      expect(aiState.activeStreamingCount).toBe(0);
      // 5. done promise rejects with AI_REQUEST_CANCELLED
      await expect(handle.done).rejects.toMatchObject({ code: "AI_REQUEST_CANCELLED" });
    });

    it("Test B: startSummary immediate cancellation before hydration", async () => {
      aiState.activeProfile = null;
      let resolveHydration;
      const hydrationPromise = new Promise((resolve) => {
        resolveHydration = resolve;
      });
      callBackend.mockImplementation(async (cmd) => {
        if (cmd === "getAiEndpointStatus") return hydrationPromise;
        return {};
      });

      const handle = aiController.startSummary("text content", "short");
      expect(handle).toBeDefined();
      expect(aiController._pendingBootstraps.has(handle.requestId)).toBe(true);
      expect(aiController._requests.has(handle.requestId)).toBe(false);

      // Caller cancels immediately before hydration resolves
      await handle.cancel();

      expect(aiController._pendingBootstraps.has(handle.requestId)).toBe(false);
      expect(aiController._requests.has(handle.requestId)).toBe(false);

      // Now hydration resolves
      const profile = {
        policy_version: 1,
        mode: "Loopback",
        canonical_origin: "http://127.0.0.1:11434",
        endpoint_fingerprint: "fp_hydrated_test",
      };
      resolveHydration({ profile });
      await vi.waitFor(() => aiState.isHydrated);

      // Assertions:
      expect(callBackend).not.toHaveBeenCalledWith("summarizeSelectionOrPage", expect.anything());
      expect(callBackend).not.toHaveBeenCalledWith("cancelAiRequest", expect.anything());
      expect(aiController._requests.size).toBe(0);
      expect(aiController._pendingBootstraps.size).toBe(0);
      expect(aiState.activeStreamingCount).toBe(0);
      await expect(handle.done).rejects.toMatchObject({ code: "AI_REQUEST_CANCELLED" });
    });

    it("Test C: dispose during pending hydration prevents subsequent request bootstrap", async () => {
      aiState.activeProfile = null;
      let resolveHydration;
      const hydrationPromise = new Promise((resolve) => {
        resolveHydration = resolve;
      });
      callBackend.mockImplementation(async (cmd) => {
        if (cmd === "getAiEndpointStatus") return hydrationPromise;
        return {};
      });

      const handle = aiController.startQueryDocumentRag("s1", 1, "Question");
      expect(aiController._pendingBootstraps.has(handle.requestId)).toBe(true);

      // Controller is disposed while hydration is pending
      aiController.dispose();

      expect(aiController._pendingBootstraps.size).toBe(0);
      expect(aiController._requests.size).toBe(0);
      await expect(handle.done).rejects.toMatchObject({ code: "AI_CONTROLLER_DISPOSED" });

      // Now hydration resolves
      const profile = {
        policy_version: 1,
        mode: "Loopback",
        canonical_origin: "http://127.0.0.1:11434",
        endpoint_fingerprint: "fp_hydrated_test",
      };
      resolveHydration({ profile });
      await vi.waitFor(() => aiState.isHydrated);

      // Backend call is NEVER initiated
      expect(callBackend).not.toHaveBeenCalledWith("queryDocumentRag", expect.anything());
      expect(aiController._requests.size).toBe(0);
      expect(aiState.activeStreamingCount).toBe(0);
    });

    it("Test D: cancel idempotency on handle produces single terminal transition", async () => {
      const handle = aiController.startSummary("text", "short");
      expect(handle).toBeDefined();

      callBackend.mockResolvedValue({});

      // First cancel
      await handle.cancel();
      expect(callBackend).toHaveBeenCalledWith("cancelAiRequest", expect.anything());
      expect(aiController._requests.size).toBe(0);

      const callCountAfterFirst = callBackend.mock.calls.length;

      // Repeated cancel
      await handle.cancel();
      await handle.cancel();

      // No additional backend cancel calls made
      expect(callBackend.mock.calls.length).toBe(callCountAfterFirst);
      await expect(handle.done).rejects.toMatchObject({ code: "AI_REQUEST_CANCELLED" });
    });

    it("Test E: normal hydrated execution happy path works without regression", async () => {
      let resolveBackend;
      const backendPromise = new Promise((res) => {
        resolveBackend = res;
      });
      callBackend.mockImplementation(async (cmd) => {
        if (cmd === "queryDocumentRag") return backendPromise;
        return {};
      });

      const chunks = [];
      const handle = aiController.startQueryDocumentRag("s1", 1, "prompt", [], (c) => chunks.push(c));
      expect(handle).toBeDefined();
      expect(aiController._requests.has(handle.requestId)).toBe(true);
      expect(aiState.activeStreamingCount).toBe(1);

      // Stream delta events
      aiController.handleStreamEvent({
        schemaVersion: 1,
        requestId: handle.requestId,
        sequence: 1,
        kind: "delta",
        delta: "chunk1 ",
        endpointFingerprint: aiState.activeProfile.endpoint_fingerprint,
        terminal: false,
      });
      aiController.handleStreamEvent({
        schemaVersion: 1,
        requestId: handle.requestId,
        sequence: 2,
        kind: "delta",
        delta: "chunk2",
        endpointFingerprint: aiState.activeProfile.endpoint_fingerprint,
        terminal: false,
      });

      expect(chunks).toEqual(["chunk1 ", "chunk2"]);

      // Complete stream
      aiController.handleStreamEvent({
        schemaVersion: 1,
        requestId: handle.requestId,
        sequence: 3,
        kind: "completed",
        result: { done: true },
        endpointFingerprint: aiState.activeProfile.endpoint_fingerprint,
        terminal: true,
      });

      resolveBackend({});
      const res = await handle.done;
      expect(res).toEqual({ done: true });
      expect(aiController._requests.size).toBe(0);
      expect(aiState.activeStreamingCount).toBe(0);
    });
  });

  it("disposes cleanly, cancels active requests, and ignores post-dispose events", () => {
    const handle = aiController.startSummary("test text", "short");
    handle.done.catch(() => {});
    const reqId = handle.requestId;

    aiController.dispose();
    // Safe double-dispose
    aiController.dispose();

    aiController.handleStreamEvent({
      schemaVersion: 1,
      requestId: reqId,
      sequence: 1,
      kind: "delta",
      delta: "Post-dispose segment",
      endpointFingerprint: aiState.activeProfile.endpoint_fingerprint,
      terminal: false,
    });

    expect(aiState.summaryStreamingText).toBe("");
  });
});
