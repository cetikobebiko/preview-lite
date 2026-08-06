import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { evaluateRenderGuard } from "./services/app/renderGuard.js";

const appSource = readFileSync(resolve(process.cwd(), "src/App.svelte"), "utf8");
const documentOpenControllerSource = readFileSync(
  resolve(process.cwd(), "src/services/app/documentOpenController.js"),
  "utf8",
);
const documentRuntimeSource = readFileSync(resolve(process.cwd(), "src/services/app/documentRuntime.js"), "utf8");
const applicationRuntimeSource = readFileSync(resolve(process.cwd(), "src/services/app/applicationRuntime.js"), "utf8");

function createDeferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("App render session lifecycle static checks", () => {
  it("sets stable render, document session, and async prompt contexts exactly once", () => {
    expect(appSource.match(/\bsetContext\s*\(/g)).toHaveLength(3);
    expect(appSource).toMatch(
      /const renderSession = createRenderSession\(\);\s*setContext\(RENDER_SESSION_CONTEXT, renderSession\);/,
    );
    expect(appSource.match(/\bcreateRenderSession\s*\(/g)).toHaveLength(1);
    expect(appSource).toMatch(
      /const documentSession = createDocumentSessionStore\(\);\s*setContext\(DOCUMENT_SESSION_CONTEXT, documentSession\);/,
    );
    expect(appSource).toMatch(
      /const asyncPrompt = createAsyncPromptCoordinator\(\);\s*setContext\(ASYNC_PROMPT_CONTEXT, asyncPrompt\);/,
    );
  });

  it("delegates document open transitions to the tested coordinator", () => {
    expect(appSource).toContain("createDocumentRuntime({");
    expect(documentRuntimeSource).toContain("createDocumentOpenController({");
    expect(documentOpenControllerSource).toContain("createDocumentOpenCoordinator({");
    expect(documentOpenControllerSource).toContain("coordinator.requestPdf(filePath)");
    expect(documentOpenControllerSource).toContain("coordinator.requestImage(filePath)");
  });

  it("stops the coordinator before asking the viewer owner to deactivate", () => {
    expect(applicationRuntimeSource).toContain("const coordinatorStop = documentRuntime.coordinator.stop();");
    expect(applicationRuntimeSource).toContain(
      "viewerRuntime.deactivatePdf(closedGeneration)",
    );
    expect(applicationRuntimeSource).not.toContain("renderActivationQueue");
  });

  it("bounds automatic commit and precommit recovery retries to one attempt id", () => {
    expect(documentOpenControllerSource).toContain(
      "if (automaticCommitRetryAttemptId === attemptId) return;",
    );
    expect(documentOpenControllerSource).toContain(
      "automaticCommitRetryAttemptId = attemptId;",
    );
    expect(documentOpenControllerSource).toContain(
      "if (automaticRecoveryRetryAttemptId === attemptId) return;",
    );
    expect(documentOpenControllerSource).toContain(
      "automaticRecoveryRetryAttemptId = attemptId;",
    );
    expect(documentOpenControllerSource).not.toMatch(
      /retryReconciliation\(\)[\s\S]{0,180}retryReconciliation\(\)/,
    );
  });
});

describe("App production evaluateRenderGuard async stale lifecycle", () => {
  it("PDF session A isteği başlar → session B aktif olur → A sonucu resolve edilir → A sonucu uygulanmaz", async () => {
    let currentSessionId = "session-A";
    const capturedSessionId = currentSessionId;
    const capturedGeneration = 1;
    const currentGeneration = 1;
    let activeRequest = 1;
    const request = 1;

    const deferredWork = createDeferred();

    const asyncRenderJob = (async () => {
      const payload = await deferredWork.promise;
      const isValid = evaluateRenderGuard({
        isMounted: true,
        request,
        activeRequest,
        isPdf: true,
        capturedSessionId,
        currentSessionId,
        capturedGeneration,
        currentGeneration,
        capturedImagePath: null,
        currentImagePath: null,
      });
      return { applied: isValid, payload };
    })();

    // Session B becomes active before A resolves
    currentSessionId = "session-B";

    deferredWork.resolve("pdf-data-A");
    const result = await asyncRenderJob;

    expect(result.applied).toBe(false);
  });

  it("Generation N isteği başlar → aynı session N+1 olur → N sonucu resolve edilir → N sonucu uygulanmaz", async () => {
    const currentSessionId = "session-A";
    const capturedSessionId = currentSessionId;
    const capturedGeneration = 1;
    let currentGeneration = 1;
    const activeRequest = 1;
    const request = 1;

    const deferredWork = createDeferred();

    const asyncRenderJob = (async () => {
      const payload = await deferredWork.promise;
      const isValid = evaluateRenderGuard({
        isMounted: true,
        request,
        activeRequest,
        isPdf: true,
        capturedSessionId,
        currentSessionId,
        capturedGeneration,
        currentGeneration,
        capturedImagePath: null,
        currentImagePath: null,
      });
      return { applied: isValid, payload };
    })();

    // Generation advances to N+1 (2) before N resolves
    currentGeneration = 2;

    deferredWork.resolve("pdf-data-gen-1");
    const result = await asyncRenderJob;

    expect(result.applied).toBe(false);
  });

  it("Image isteği başlar → PDF aktif olur → image sonucu resolve edilir → image sonucu uygulanmaz", async () => {
    let isPdf = false;
    const capturedImagePath = "/tmp/image.png";
    let currentImagePath = "/tmp/image.png";
    const activeRequest = 1;
    const request = 1;

    const deferredWork = createDeferred();

    const asyncRenderJob = (async () => {
      const payload = await deferredWork.promise;
      const isValid = evaluateRenderGuard({
        isMounted: true,
        request,
        activeRequest,
        isPdf,
        capturedSessionId: null,
        currentSessionId: isPdf ? "session-A" : null,
        capturedGeneration: null,
        currentGeneration: isPdf ? 1 : null,
        capturedImagePath,
        currentImagePath,
      });
      return { applied: isValid, payload };
    })();

    // Mode switches to PDF before image resolves
    isPdf = true;
    currentImagePath = "";

    deferredWork.resolve("image-rgba-data");
    const result = await asyncRenderJob;

    expect(result.applied).toBe(false);
  });

  it("PDF isteği başlar → image aktif olur → PDF sonucu resolve edilir → PDF sonucu uygulanmaz", async () => {
    let isPdf = true;
    const capturedSessionId = "session-A";
    let currentSessionId = "session-A";
    const capturedGeneration = 1;
    let currentGeneration = 1;
    let currentImagePath = "";
    const activeRequest = 1;
    const request = 1;

    const deferredWork = createDeferred();

    const asyncRenderJob = (async () => {
      const payload = await deferredWork.promise;
      const isValid = evaluateRenderGuard({
        isMounted: true,
        request,
        activeRequest,
        isPdf,
        capturedSessionId,
        currentSessionId,
        capturedGeneration,
        currentGeneration,
        capturedImagePath: null,
        currentImagePath,
      });
      return { applied: isValid, payload };
    })();

    // Mode switches to Image before PDF resolves
    isPdf = false;
    currentSessionId = null;
    currentGeneration = 0;
    currentImagePath = "/tmp/photo.jpg";

    deferredWork.resolve("pdf-page-data");
    const result = await asyncRenderJob;

    expect(result.applied).toBe(false);
  });

  it("Request ID guard, component-mounted durumu ve diğer tüm parametrelerin production guard'ıyla doğrulandığını gösterir", () => {
    // Component unmounted -> false
    expect(
      evaluateRenderGuard({
        isMounted: false,
        request: 1,
        activeRequest: 1,
        isPdf: true,
        capturedSessionId: "session-A",
        currentSessionId: "session-A",
        capturedGeneration: 1,
        currentGeneration: 1,
        capturedImagePath: null,
        currentImagePath: null,
      }),
    ).toBe(false);

    // Request ID mismatch -> false
    expect(
      evaluateRenderGuard({
        isMounted: true,
        request: 1,
        activeRequest: 2,
        isPdf: true,
        capturedSessionId: "session-A",
        currentSessionId: "session-A",
        capturedGeneration: 1,
        currentGeneration: 1,
        capturedImagePath: null,
        currentImagePath: null,
      }),
    ).toBe(false);

    // All match for PDF -> true
    expect(
      evaluateRenderGuard({
        isMounted: true,
        request: 1,
        activeRequest: 1,
        isPdf: true,
        capturedSessionId: "session-A",
        currentSessionId: "session-A",
        capturedGeneration: 1,
        currentGeneration: 1,
        capturedImagePath: null,
        currentImagePath: null,
      }),
    ).toBe(true);

    // All match for Image -> true
    expect(
      evaluateRenderGuard({
        isMounted: true,
        request: 1,
        activeRequest: 1,
        isPdf: false,
        capturedSessionId: null,
        currentSessionId: null,
        capturedGeneration: null,
        currentGeneration: null,
        capturedImagePath: "/tmp/img.png",
        currentImagePath: "/tmp/img.png",
      }),
    ).toBe(true);
  });
});
