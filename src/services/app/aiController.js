import { listen } from "@tauri-apps/api/event";
import { aiState } from "../../stores/aiState.svelte.js";
import * as backendAi from "../backend/ai.js";

class AiController {
  constructor() {
    this._unsubStream = null;
    this._disposed = false;
    this._hydrationPromise = null;
    this._pendingBootstraps = new Map();
    this._requests = new Map();
    this._lastSequenceByRequest = new Map();
    this._getGuard = null;
    this._isGuardCurrent = null;
    this._initStorageMigration();
    this._setupStreamListener();
  }

  init({ getGuard, isGuardCurrent } = {}) {
    this._disposed = false;
    this._pendingBootstraps.clear();
    this._requests.clear();
    this._lastSequenceByRequest.clear();
    if (getGuard) this._getGuard = getGuard;
    if (isGuardCurrent) this._isGuardCurrent = isGuardCurrent;
    this._setupStreamListener();
  }

  dispose() {
    if (this._disposed) return;
    this._disposed = true;

    if (this._unsubStream) {
      const unsub = this._unsubStream;
      this._unsubStream = null;
      try {
        if (typeof unsub === "function") {
          unsub();
        } else if (unsub && typeof unsub.then === "function") {
          unsub.then((fn) => {
            if (typeof fn === "function") fn();
          });
        }
      } catch {
        // Ignore unlisten error
      }
    }

    for (const boot of Array.from(this._pendingBootstraps.values())) {
      boot.cancelled = true;
      if (boot.doneReject) {
        boot.doneReject({ code: "AI_CONTROLLER_DISPOSED", message: "AI controller disposed" });
      }
    }
    this._pendingBootstraps.clear();

    for (const ctx of Array.from(this._requests.values())) {
      try {
        if (typeof ctx.cancel === "function") {
          void ctx.cancel();
        }
      } catch {
        // Ignore cancel error on dispose
      }
      if (!ctx.isTerminal && ctx.doneReject) {
        ctx.doneReject({ code: "AI_CONTROLLER_DISPOSED", message: "AI controller disposed" });
      }
    }
    this._requests.clear();
    this._lastSequenceByRequest.clear();
    aiState.isIndexing = false;
    aiState.activeStreamingCount = 0;
  }

  async ensureHydrated() {
    if (aiState.activeProfile?.endpoint_fingerprint || aiState.activeProfile?.endpointFingerprint) {
      return aiState.activeProfile;
    }
    if (!this._hydrationPromise) {
      this._hydrationPromise = this.fetchEndpointStatus().finally(() => {
        this._hydrationPromise = null;
      });
    }
    return this._hydrationPromise;
  }

  _checkAuthorization() {
    const profile = aiState.activeProfile;
    if (!profile) {
      return {
        code: "AI_ENDPOINT_INVALID",
        message: "Aktif AI endpoint profili bulunamadı.",
      };
    }
    const mode = profile.mode;
    const origin = profile.canonical_origin || profile.canonicalOrigin || "";

    if (mode === "Loopback") {
      return null;
    }
    if (mode === "RemotePublic") {
      if (!aiState.remoteConsent) {
        return {
          code: "AI_ENDPOINT_REMOTE_CONSENT_REQUIRED",
          message: `Uzak AI endpoint kullanımı için onay gereklidir: ${origin}`,
        };
      }
      return null;
    }
    if (mode === "PrivateNetwork") {
      if (!aiState.remoteConsent) {
        return {
          code: "AI_ENDPOINT_REMOTE_CONSENT_REQUIRED",
          message: `Uzak AI endpoint kullanımı için onay gereklidir: ${origin}`,
        };
      }
      if (!aiState.privateConsent) {
        return {
          code: "AI_ENDPOINT_PRIVATE_NETWORK_CONSENT_REQUIRED",
          message: `Özel ağ AI endpoint kullanımı için onay gereklidir: ${origin}`,
        };
      }
      return null;
    }

    return {
      code: "AI_ENDPOINT_INVALID",
      message: `Bilinmeyen endpoint modu: ${mode}`,
    };
  }

  _initStorageMigration() {
    try {
      const savedSettings = localStorage.getItem("ai_settings_v1");
      if (savedSettings) {
        const parsed = JSON.parse(savedSettings);
        if (parsed && parsed.schemaVersion === 1) {
          const modelRegex = /^[a-zA-Z0-9_.:/-]+$/;
          if (
            typeof parsed.endpointDraft === "string" &&
            parsed.endpointDraft.length > 0 &&
            parsed.endpointDraft.length <= 2048 &&
            (parsed.endpointDraft.startsWith("http://") || parsed.endpointDraft.startsWith("https://"))
          ) {
            aiState.endpointDraft = parsed.endpointDraft;
          }
          if (
            typeof parsed.llmModel === "string" &&
            parsed.llmModel.length > 0 &&
            parsed.llmModel.length <= 128 &&
            modelRegex.test(parsed.llmModel)
          ) {
            aiState.llmModel = parsed.llmModel;
          }
          if (
            typeof parsed.embedModel === "string" &&
            parsed.embedModel.length > 0 &&
            parsed.embedModel.length <= 128 &&
            modelRegex.test(parsed.embedModel)
          ) {
            aiState.embedModel = parsed.embedModel;
          }
        }
      }
    } catch {
      // Storage access fail-safe
    }
  }

  persistSettings() {
    try {
      const payload = {
        schemaVersion: 1,
        endpointDraft: aiState.endpointDraft,
        llmModel: aiState.llmModel,
        embedModel: aiState.embedModel,
      };
      localStorage.setItem("ai_settings_v1", JSON.stringify(payload));
    } catch {
      // Persistence error ignored
    }
  }

  async _setupStreamListener() {
    if (this._unsubStream || this._disposed) return;
    try {
      const unsub = await listen("ai_stream_event", (event) => {
        if (this._disposed) return;
        this.handleStreamEvent(event.payload);
      });
      if (this._disposed) {
        if (typeof unsub === "function") unsub();
      } else {
        this._unsubStream = unsub;
      }
    } catch {
      // Event listener fallback for vitest environment
    }
  }

  handleStreamEvent(evt) {
    if (this._disposed || !evt || evt.schemaVersion !== 1) return;

    const { requestId, sequence, kind, delta, result, error, terminal, endpointFingerprint } = evt;

    const ctx = this._requests.get(requestId);
    if (!ctx) return;

    const activeFp = aiState.activeProfile?.endpoint_fingerprint || aiState.activeProfile?.endpointFingerprint;
    if (endpointFingerprint && ctx.endpointFingerprint && endpointFingerprint !== ctx.endpointFingerprint) {
      return;
    }
    if (activeFp && endpointFingerprint && endpointFingerprint !== activeFp) {
      return;
    }

    if (ctx.capturedGuard && this._isGuardCurrent) {
      if (!this._isGuardCurrent(ctx.capturedGuard)) {
        return;
      }
    }

    const lastSeq = this._lastSequenceByRequest.get(requestId) || 0;
    if (sequence <= lastSeq) {
      this.handleStreamError(requestId, {
        code: "AI_STREAM_SEQUENCE_ERROR",
        message: `Sequence regression or duplicate detected: got ${sequence}, last was ${lastSeq}`,
      });
      return;
    }
    this._lastSequenceByRequest.set(requestId, sequence);

    for (const sub of ctx.subscribers) {
      try {
        sub(evt);
      } catch {
        // Subscriber error ignored
      }
    }

    if (terminal) {
      this._finishRequest(requestId, kind, result, error);
    }
  }

  handleStreamError(requestId, err) {
    aiState.error = err;
    const ctx = this._requests.get(requestId);
    if (ctx) {
      const evt = {
        schemaVersion: 1,
        requestId,
        kind: "failed",
        error: err,
        terminal: true,
      };
      for (const sub of ctx.subscribers) {
        try {
          sub(evt);
        } catch {
          // Subscriber error ignored
        }
      }
      this._finishRequest(requestId, "failed", null, err);
    }
  }

  _finishRequest(requestId, kind, result, error) {
    const ctx = this._requests.get(requestId);
    if (!ctx || ctx.isTerminal) return;

    ctx.isTerminal = true;
    this._requests.delete(requestId);
    this._lastSequenceByRequest.delete(requestId);

    if (ctx.isStreaming !== false) {
      aiState.decrementStreaming();
    }

    if (kind === "failed") {
      const errObj = error || { code: "AI_REQUEST_FAILED", message: "Request failed" };
      aiState.error = errObj;
      if (ctx.doneReject) ctx.doneReject(errObj);
    } else if (kind === "cancelled") {
      const cancelErr = { code: "AI_REQUEST_CANCELLED", message: "Request cancelled" };
      if (ctx.doneReject) ctx.doneReject(cancelErr);
    } else {
      if (ctx.doneResolve) ctx.doneResolve(result);
    }
  }

  async configureEndpoint(url, remoteApproved = false, privateApproved = false) {
    aiState.clearError();
    await this.cancelActiveRequest();
    aiState.clearIndexedMetadata();
    try {
      const profile = await backendAi.configureAiEndpoint(url, remoteApproved, privateApproved);
      aiState.activeProfile = profile;
      aiState.remoteConsent = remoteApproved;
      aiState.privateConsent = privateApproved;
      this.persistSettings();
      return profile;
    } catch (err) {
      aiState.error = err;
      throw err;
    }
  }

  async fetchEndpointStatus() {
    try {
      const statusDto = await backendAi.getAiEndpointStatus();
      const profile = statusDto?.profile || statusDto;
      aiState.activeProfile = profile;
      if (statusDto?.consent) {
        aiState.remoteConsent = Boolean(statusDto.consent.remote_approved);
        aiState.privateConsent = Boolean(statusDto.consent.private_network_approved);
      }
      if (profile?.canonical_origin) {
        aiState.endpointDraft = profile.canonical_origin;
      }
      return profile;
    } catch (err) {
      aiState.error = err;
      throw err;
    }
  }

  async clearEndpointConfig() {
    await this.cancelActiveRequest();
    aiState.clearIndexedMetadata();
    try {
      const profile = await backendAi.clearAiEndpointConfig();
      aiState.activeProfile = profile;
      aiState.remoteConsent = false;
      aiState.privateConsent = false;
      return profile;
    } catch (err) {
      aiState.error = err;
      throw err;
    }
  }

  setEmbedModel(model) {
    if (aiState.embedModel !== model) {
      aiState.embedModel = model;
      aiState.clearIndexedMetadata();
      for (const [reqId, ctx] of Array.from(this._requests.entries())) {
        if (ctx.operation === "index") {
          void this.cancelRequest(reqId, ctx.sessionId, ctx.expectedContentGeneration, null);
        }
      }
      this.persistSettings();
    }
  }

  setLlmModel(model) {
    if (aiState.llmModel !== model) {
      aiState.llmModel = model;
      this.persistSettings();
    }
  }

  async buildSemanticIndex(
    sessionId,
    expectedContentGeneration,
    chunkSize = aiState.chunkSize,
    chunkOverlap = aiState.chunkOverlap,
  ) {
    if (this._disposed) {
      throw { code: "AI_CONTROLLER_DISPOSED", message: "AI controller disposed" };
    }
    await this.ensureHydrated();
    if (this._disposed) {
      throw { code: "AI_CONTROLLER_DISPOSED", message: "AI controller disposed" };
    }
    const authError = this._checkAuthorization();
    if (authError) {
      aiState.error = authError;
      throw authError;
    }

    const requestId = `req_${crypto.randomUUID()}`;
    aiState.isIndexing = true;
    aiState.clearError();

    const activeFp =
      aiState.activeProfile?.endpoint_fingerprint ||
      aiState.activeProfile?.endpointFingerprint ||
      "";

    const ctx = {
      requestId,
      sessionId,
      expectedContentGeneration,
      operation: "index",
      isStreaming: false,
      endpointFingerprint: activeFp,
      cancel: () =>
        this.cancelRequest(
          requestId,
          sessionId,
          expectedContentGeneration,
          null,
        ),
    };
    this._requests.set(requestId, ctx);

    try {
      const res = await backendAi.buildSemanticIndex(
        sessionId,
        expectedContentGeneration,
        chunkSize,
        chunkOverlap,
        aiState.embedModel,
        requestId,
      );

      aiState.setIndexedMetadata({
        sessionId,
        generation: expectedContentGeneration,
        embedModel: aiState.embedModel,
        endpointFingerprint: activeFp,
        chunkSize,
        chunkOverlap,
        chunkCount: (res && res.chunkCount) || 0,
        totalChars: (res && res.totalChars) || 0,
      });

      return res;
    } catch (err) {
      aiState.clearIndexedMetadata();
      aiState.error = err;
      throw err;
    } finally {
      aiState.isIndexing = false;
      this._requests.delete(requestId);
    }
  }

  async semanticSearch(
    sessionId,
    expectedContentGeneration,
    query,
    topK = 3,
    chunkSize = aiState.chunkSize,
    chunkOverlap = aiState.chunkOverlap,
  ) {
    if (this._disposed) {
      throw { code: "AI_CONTROLLER_DISPOSED", message: "AI controller disposed" };
    }
    await this.ensureHydrated();
    if (this._disposed) {
      throw { code: "AI_CONTROLLER_DISPOSED", message: "AI controller disposed" };
    }
    const authError = this._checkAuthorization();
    if (authError) {
      aiState.error = authError;
      throw authError;
    }

    const requestId = `req_${crypto.randomUUID()}`;
    aiState.clearError();
    try {
      return await backendAi.semanticSearch(
        sessionId,
        expectedContentGeneration,
        query,
        topK,
        aiState.embedModel,
        chunkSize,
        chunkOverlap,
        requestId,
      );
    } catch (err) {
      aiState.error = err;
      throw err;
    }
  }

  startQueryDocumentRag(sessionId, expectedContentGeneration, prompt, history = [], onChunk = null) {
    const requestId = `req_${crypto.randomUUID()}`;
    aiState.clearError();

    let capturedGuard = null;
    if (this._getGuard) {
      capturedGuard = this._getGuard({ includeContentGeneration: true, targetSessionId: sessionId });
    }

    let doneResolve, doneReject;
    const donePromise = new Promise((resolve, reject) => {
      doneResolve = resolve;
      doneReject = reject;
    });

    const subscribers = new Set();
    if (onChunk) {
      subscribers.add((evt) => {
        if (evt.kind === "delta" && evt.delta) {
          onChunk(evt.delta);
        }
      });
    }

    const cancel = () => {
      // 1. If currently in pending bootstrap:
      if (this._pendingBootstraps.has(requestId)) {
        const boot = this._pendingBootstraps.get(requestId);
        if (boot) boot.cancelled = true;
        this._pendingBootstraps.delete(requestId);
        doneReject({ code: "AI_REQUEST_CANCELLED", message: "Request cancelled" });
        return Promise.resolve();
      }
      // 2. If active in _requests:
      if (this._requests.has(requestId)) {
        return this.cancelRequest(requestId, sessionId, expectedContentGeneration, null);
      }
      // 3. If already terminal / unknown: idempotent no-op
      return Promise.resolve();
    };

    // If profile is already hydrated, perform synchronous authorization and active request registration
    if (aiState.activeProfile) {
      if (this._disposed) {
        doneReject({ code: "AI_CONTROLLER_DISPOSED", message: "AI controller disposed" });
        return {
          requestId,
          cancel,
          done: donePromise,
          subscribe: (cb) => {
            subscribers.add(cb);
            return () => subscribers.delete(cb);
          },
        };
      }

      const authError = this._checkAuthorization();
      if (authError) {
        aiState.error = authError;
        doneReject(authError);
        return {
          requestId,
          cancel,
          done: donePromise,
          subscribe: (cb) => {
            subscribers.add(cb);
            return () => subscribers.delete(cb);
          },
        };
      }

      aiState.incrementStreaming();
      const activeFp =
        aiState.activeProfile?.endpoint_fingerprint ||
        aiState.activeProfile?.endpointFingerprint ||
        "";
      const ctx = {
        requestId,
        sessionId,
        expectedContentGeneration,
        operation: "chat",
        isStreaming: true,
        endpointFingerprint: activeFp,
        capturedGuard,
        subscribers,
        doneResolve,
        doneReject,
        donePromise,
        isTerminal: false,
        cancel,
      };
      this._requests.set(requestId, ctx);

      void (async () => {
        try {
          await backendAi.queryDocumentRag(
            sessionId,
            expectedContentGeneration,
            prompt,
            history,
            aiState.llmModel,
            aiState.embedModel,
            aiState.chunkSize,
            aiState.chunkOverlap,
            requestId,
          );
        } catch (err) {
          if (this._requests.has(requestId)) {
            this._finishRequest(requestId, "failed", null, err);
          } else {
            aiState.error = err;
            doneReject(err);
          }
        }
      })();

      return {
        requestId,
        cancel,
        done: donePromise,
        subscribe: (cb) => {
          subscribers.add(cb);
          return () => subscribers.delete(cb);
        },
      };
    }

    // Otherwise, register to private bootstrap and await hydration
    const bootstrapCtx = {
      requestId,
      cancelled: false,
      doneResolve,
      doneReject,
      subscribers,
    };
    this._pendingBootstraps.set(requestId, bootstrapCtx);

    void (async () => {
      try {
        if (this._disposed || bootstrapCtx.cancelled) return;

        await this.ensureHydrated();
        if (this._disposed || bootstrapCtx.cancelled) return;

        const authError = this._checkAuthorization();
        if (authError) {
          this._pendingBootstraps.delete(requestId);
          aiState.error = authError;
          doneReject(authError);
          return;
        }

        if (this._disposed || bootstrapCtx.cancelled) return;

        // Atomic handoff from bootstrap to active _requests
        this._pendingBootstraps.delete(requestId);

        aiState.incrementStreaming();
        const activeFp =
          aiState.activeProfile?.endpoint_fingerprint ||
          aiState.activeProfile?.endpointFingerprint ||
          "";
        const ctx = {
          requestId,
          sessionId,
          expectedContentGeneration,
          operation: "chat",
          isStreaming: true,
          endpointFingerprint: activeFp,
          capturedGuard,
          subscribers,
          doneResolve,
          doneReject,
          donePromise,
          isTerminal: false,
          cancel,
        };
        this._requests.set(requestId, ctx);

        await backendAi.queryDocumentRag(
          sessionId,
          expectedContentGeneration,
          prompt,
          history,
          aiState.llmModel,
          aiState.embedModel,
          aiState.chunkSize,
          aiState.chunkOverlap,
          requestId,
        );
      } catch (err) {
        this._pendingBootstraps.delete(requestId);
        if (this._requests.has(requestId)) {
          this._finishRequest(requestId, "failed", null, err);
        } else if (!bootstrapCtx.cancelled) {
          aiState.error = err;
          doneReject(err);
        }
      }
    })();

    return {
      requestId,
      cancel,
      done: donePromise,
      subscribe: (cb) => {
        subscribers.add(cb);
        return () => subscribers.delete(cb);
      },
    };
  }

  queryDocumentRag(sessionId, expectedContentGeneration, prompt, history = [], onChunk = null) {
    return this.startQueryDocumentRag(
      sessionId,
      expectedContentGeneration,
      prompt,
      history,
      onChunk,
    );
  }

  startSummary(textContent, targetLength = "short", onChunk = null) {
    const requestId = `req_${crypto.randomUUID()}`;
    const callerAttemptId = `attempt_${crypto.randomUUID()}`;

    aiState.clearError();

    let doneResolve, doneReject;
    const donePromise = new Promise((resolve, reject) => {
      doneResolve = resolve;
      doneReject = reject;
    });

    const subscribers = new Set();
    subscribers.add((evt) => {
      if (evt.kind === "delta" && evt.delta) {
        aiState.summaryStreamingText += evt.delta;
        if (onChunk) onChunk(evt.delta);
      }
    });

    const cancel = () => {
      // 1. If currently in pending bootstrap:
      if (this._pendingBootstraps.has(requestId)) {
        const boot = this._pendingBootstraps.get(requestId);
        if (boot) boot.cancelled = true;
        this._pendingBootstraps.delete(requestId);
        doneReject({ code: "AI_REQUEST_CANCELLED", message: "Request cancelled" });
        return Promise.resolve();
      }
      // 2. If active in _requests:
      if (this._requests.has(requestId)) {
        return this.cancelRequest(requestId, null, null, callerAttemptId);
      }
      // 3. If already terminal / unknown: idempotent no-op
      return Promise.resolve();
    };

    if (aiState.activeProfile) {
      if (this._disposed) {
        doneReject({ code: "AI_CONTROLLER_DISPOSED", message: "AI controller disposed" });
        return {
          requestId,
          callerAttemptId,
          cancel,
          done: donePromise,
          subscribe: (cb) => {
            subscribers.add(cb);
            return () => subscribers.delete(cb);
          },
        };
      }

      const authError = this._checkAuthorization();
      if (authError) {
        aiState.error = authError;
        doneReject(authError);
        return {
          requestId,
          callerAttemptId,
          cancel,
          done: donePromise,
          subscribe: (cb) => {
            subscribers.add(cb);
            return () => subscribers.delete(cb);
          },
        };
      }

      aiState.resetSummary();
      aiState.incrementStreaming();
      const activeFp =
        aiState.activeProfile?.endpoint_fingerprint ||
        aiState.activeProfile?.endpointFingerprint ||
        "";
      const ctx = {
        requestId,
        callerAttemptId,
        operation: "summary",
        isStreaming: true,
        endpointFingerprint: activeFp,
        capturedGuard: null,
        subscribers,
        doneResolve,
        doneReject,
        donePromise,
        isTerminal: false,
        cancel,
      };
      this._requests.set(requestId, ctx);

      void (async () => {
        try {
          await backendAi.summarizeSelectionOrPage(
            textContent,
            targetLength,
            aiState.llmModel,
            requestId,
            callerAttemptId,
          );
        } catch (err) {
          if (this._requests.has(requestId)) {
            this._finishRequest(requestId, "failed", null, err);
          } else {
            aiState.error = err;
            doneReject(err);
          }
        }
      })();

      return {
        requestId,
        callerAttemptId,
        cancel,
        done: donePromise,
        subscribe: (cb) => {
          subscribers.add(cb);
          return () => subscribers.delete(cb);
        },
      };
    }

    const bootstrapCtx = {
      requestId,
      callerAttemptId,
      cancelled: false,
      doneResolve,
      doneReject,
      subscribers,
    };
    this._pendingBootstraps.set(requestId, bootstrapCtx);

    void (async () => {
      try {
        if (this._disposed || bootstrapCtx.cancelled) return;

        await this.ensureHydrated();
        if (this._disposed || bootstrapCtx.cancelled) return;

        const authError = this._checkAuthorization();
        if (authError) {
          this._pendingBootstraps.delete(requestId);
          aiState.error = authError;
          doneReject(authError);
          return;
        }

        if (this._disposed || bootstrapCtx.cancelled) return;

        // Atomic handoff from bootstrap to active _requests
        this._pendingBootstraps.delete(requestId);

        aiState.resetSummary();
        aiState.incrementStreaming();
        const activeFp =
          aiState.activeProfile?.endpoint_fingerprint ||
          aiState.activeProfile?.endpointFingerprint ||
          "";
        const ctx = {
          requestId,
          callerAttemptId,
          operation: "summary",
          isStreaming: true,
          endpointFingerprint: activeFp,
          capturedGuard: null,
          subscribers,
          doneResolve,
          doneReject,
          donePromise,
          isTerminal: false,
          cancel,
        };
        this._requests.set(requestId, ctx);

        await backendAi.summarizeSelectionOrPage(
          textContent,
          targetLength,
          aiState.llmModel,
          requestId,
          callerAttemptId,
        );
      } catch (err) {
        this._pendingBootstraps.delete(requestId);
        if (this._requests.has(requestId)) {
          this._finishRequest(requestId, "failed", null, err);
        } else if (!bootstrapCtx.cancelled) {
          aiState.error = err;
          doneReject(err);
        }
      }
    })();

    return {
      requestId,
      callerAttemptId,
      cancel,
      done: donePromise,
      subscribe: (cb) => {
        subscribers.add(cb);
        return () => subscribers.delete(cb);
      },
    };
  }

  summarizeSelectionOrPage(textContent, targetLength = "short", onChunk = null) {
    return this.startSummary(textContent, targetLength, onChunk);
  }

  async cancelRequest(requestId, sessionId = null, expectedContentGeneration = null, callerAttemptId = null) {
    const ctx = this._requests.get(requestId);
    if (ctx && ctx.isTerminal) {
      return;
    }
    try {
      await backendAi.cancelAiRequest(requestId, sessionId, expectedContentGeneration, callerAttemptId);
      if (ctx) {
        this._finishRequest(requestId, "cancelled", null, null);
      }
    } catch (err) {
      aiState.error = err;
      throw err;
    }
  }

  async cancelActiveRequest() {
    const pendingBootstraps = Array.from(this._pendingBootstraps.values());
    for (const boot of pendingBootstraps) {
      boot.cancelled = true;
      this._pendingBootstraps.delete(boot.requestId);
      if (boot.doneReject) {
        boot.doneReject({ code: "AI_REQUEST_CANCELLED", message: "Request cancelled" });
      }
    }
    const activeRequests = Array.from(this._requests.values());
    for (const ctx of activeRequests) {
      try {
        if (typeof ctx.cancel === "function") {
          await ctx.cancel();
        }
      } catch {
        // Best effort
      }
    }
  }
}

export const aiController = new AiController();
