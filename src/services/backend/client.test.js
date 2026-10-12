import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  BackendCommandError,
  callBackend,
  callBackendProfiled,
  invokeProfiled,
} from "./client.js";
import { encodeRenderFrame } from "./renderFrameTestUtils.js";

const invokeMock = vi.hoisted(() => vi.fn());
const JOB_ID = "00000000-0000-4000-8000-000000000001";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

beforeEach(() => {
  invokeMock.mockReset();
});

describe("validated backend client", () => {
  it("rejects missing required fields before invoke", async () => {
    await expect(
      callBackend("mergePdfs", {
        filePaths: ["/tmp/a.pdf"],
        outputPath: undefined,
      }),
    ).rejects.toMatchObject({
      command: "merge_pdfs",
      message: expect.stringContaining("outputPath"),
      cause: expect.any(TypeError),
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("rejects unknown top-level and validated nested fields", async () => {
    await expect(
      callBackend("executeDocumentSanitization", {
        jobId: JOB_ID,
        sessionId: "sid",
        expectedContentGeneration: 1,
        targetPath: "/tmp/b.pdf",
        policy: {
          schemaVersion: "sanitization-policy-v1",
          profile: "custom_supported_v1",
          controls: ["incremental_revisions"],
          expectedInspectionVersion: "sanitization-inspection-v1",
          acknowledgements: {
            acknowledgeSignatureInvalidation: false,
            unexpected: true,
          },
        },
      }),
    ).rejects.toMatchObject({
      command: "execute_document_sanitization",
      message: expect.stringContaining("unexpected"),
    });

    await expect(
      callBackend("mergePdfs", {
        filePaths: ["/tmp/a.pdf"],
        outputPath: "/tmp/b.pdf",
        alias: "not allowed",
      }),
    ).rejects.toMatchObject({
      command: "merge_pdfs",
      message: expect.stringContaining("alias"),
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("removes optional undefined fields and preserves allowed null", async () => {
    invokeMock.mockResolvedValueOnce("ok");
    await callBackend("printSessionPdf", {
      jobId: JOB_ID,
      sessionId: "sid",
      expectedContentGeneration: 1,
      pageRange: null,
    });
    expect(invokeMock).toHaveBeenLastCalledWith("print_session_pdf", {
      jobId: JOB_ID,
      sessionId: "sid",
      expectedContentGeneration: 1,
      pageRange: null,
    });
  });

  it("rejects null for non-Option fields and malformed tuples", async () => {
    await expect(
      callBackend("getSessionPageCount", {
        sessionId: null,
        expectedContentGeneration: 1,
      }),
    ).rejects.toMatchObject({
      command: "get_session_page_count",
      message: expect.stringContaining("null is not allowed"),
    });

    await expect(
      callBackend("splitSessionPdf", {
        jobId: JOB_ID,
        sessionId: "sid",
        expectedContentGeneration: 1,
        ranges: [[0, 1, 2]],
        outputDir: "/tmp",
      }),
    ).rejects.toMatchObject({
      command: "split_session_pdf",
      message: expect.stringContaining("tuple of length 2"),
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("rejects integer values outside the Rust type range", async () => {
    await expect(
      callBackend("renderSessionThumbnail", {
        sessionId: "document",
        pageIndex: 4_294_967_296,
        targetWidth: 180,
        contentGeneration: 1,
        requestId: "request",
        scope: "sidebar_thumbnail",
      }),
    ).rejects.toMatchObject({
      command: "render_session_thumbnail",
      message: expect.stringContaining("4294967295"),
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it.each([
    ["generation", { contentGeneration: Number.MAX_SAFE_INTEGER + 1 }, "9007199254740991"],
    ["request ID length", { requestId: "r".repeat(129) }, "string length <= 128"],
    ["scope enum", { scope: "unknown" }, "expected one of"],
    ["target width maximum", { targetWidth: 16_385 }, "16384"],
  ])("rejects render boundary: %s", async (_label, override, message) => {
    await expect(
      callBackend("renderSessionThumbnail", {
        sessionId: "document",
        pageIndex: 0,
        targetWidth: 180,
        contentGeneration: 1,
        requestId: "request",
        scope: "sidebar_thumbnail",
        ...override,
      }),
    ).rejects.toMatchObject({
      command: "render_session_thumbnail",
      message: expect.stringContaining(message),
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("enforces scale, rotation and thumbnail consumer boundaries", async () => {
    const base = {
      sessionId: "document",
      pageIndex: 0,
      scale: 1,
      rotation: 0,
      contentGeneration: 1,
      requestId: "request",
    };
    await expect(
      callBackend("renderSessionPage", { ...base, scale: 16.01 }),
    ).rejects.toMatchObject({ message: expect.stringContaining("16") });
    await expect(
      callBackend("renderSessionPage", { ...base, rotation: 45 }),
    ).rejects.toMatchObject({ message: expect.stringContaining("rotation") });
    await expect(
      callBackend("renderSessionThumbnail", {
        sessionId: base.sessionId,
        pageIndex: base.pageIndex,
        targetWidth: 180,
        contentGeneration: base.contentGeneration,
        requestId: base.requestId,
        scope: "main_page",
      }),
    ).rejects.toMatchObject({ message: expect.stringContaining("thumbnail consumer") });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("requires a valid session identifier and allows idempotent deactivation", async () => {
    await expect(
      callBackend("setActiveSessionRenderDocument", {
        sessionId: 42,
        contentGeneration: 1,
      }),
    ).rejects.toMatchObject({
      command: "set_active_session_render_document",
      message: expect.stringContaining("expected string"),
    });
    expect(invokeMock).not.toHaveBeenCalled();

    invokeMock.mockResolvedValueOnce({ changed: false });
    await expect(
      callBackend("setActiveSessionRenderDocument", {
        sessionId: null,
        contentGeneration: 2,
      }),
    ).resolves.toEqual({ changed: false });
  });

  it("enforces required nested fields and rejects unknown nested metadata fields", async () => {
    await expect(
      callBackend("updateSessionMetadata", {
        sessionId: "sid",
        expectedContentGeneration: 1,
        metadata: {
          title: "Title",
          author: "Author",
          subject: "Subject",
          keywords: "keywords",
        },
      }),
    ).rejects.toMatchObject({
      command: "update_session_metadata",
      message: expect.stringContaining("metadata.creator"),
    });

    await expect(
      callBackend("updateSessionMetadata", {
        sessionId: "sid",
        expectedContentGeneration: 1,
        metadata: {
          title: "Title",
          author: "Author",
          subject: "Subject",
          keywords: "keywords",
          creator: "Creator",
          unsupported: true,
        },
      }),
    ).rejects.toMatchObject({
      command: "update_session_metadata",
      message: expect.stringContaining("unsupported"),
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it.each([
    ["backend string", "backend string"],
    [new Error("backend Error"), "backend Error"],
    [{ message: "backend object", code: 500 }, "backend object"],
  ])("normalizes transport failures without losing cause", async (cause, message) => {
    invokeMock.mockRejectedValueOnce(cause);

    const error = await callBackend("getSessionPageCount", {
      sessionId: "sid",
      expectedContentGeneration: 1,
    }).catch((caught) => caught);

    expect(error).toBeInstanceOf(BackendCommandError);
    expect(error).toMatchObject({
      command: "get_session_page_count",
      message,
      cause,
    });
  });

  it("does not wrap an already normalized error again", async () => {
    const normalized = new BackendCommandError(
      "get_session_page_count",
      "already normalized",
      "original",
    );
    invokeMock.mockRejectedValueOnce(normalized);

    const caught = await callBackend("getSessionPageCount", {
      sessionId: "sid",
      expectedContentGeneration: 1,
    }).catch((error) => error);

    expect(caught).toBe(normalized);
  });

  it.each([
    "PDFIUM_WORKER_UNAVAILABLE",
    "PDFIUM_WORKER_SPAWN_FAILED",
    "PDFIUM_WORKER_INTERNAL",
    "PDFIUM_WORKER_LOAD_FAILED",
    "PDFIUM_WORKER_RENDER_FAILED",
    "PDFIUM_WORKER_CRASHED",
    "PDFIUM_WORKER_TIMEOUT",
    "PDFIUM_WORKER_PROTOCOL_MISMATCH",
    "PDFIUM_WORKER_MESSAGE_TOO_LARGE",
    "PDFIUM_DOCUMENT_QUARANTINED",
    "PDFIUM_PAGE_QUARANTINED",
  ])("preserves typed worker failure code and message: %s", async (code) => {
    const cause = {
      code,
      message: "Belge güvenli şekilde işlenemedi; işlem tamamlanmadı.",
    };
    invokeMock.mockRejectedValueOnce(cause);

    const caught = await callBackend("getSessionPageCount", {
      sessionId: "sid",
      expectedContentGeneration: 1,
    }).catch((error) => error);

    expect(caught).toMatchObject({
      command: "get_session_page_count",
      code,
      message: cause.message,
      cause,
    });
    expect(caught.toString()).toContain(
      "Belge güvenli şekilde işlenemedi ve işlem tamamlanmadı.",
    );
  });

  it("treats normal worker cancellation as cancellation rather than a safety failure", async () => {
    const cause = {
      code: "PDFIUM_WORKER_CANCELLED",
      message: "İşlem kullanıcı tarafından iptal edildi.",
    };
    invokeMock.mockRejectedValueOnce(cause);

    const caught = await callBackend("getSessionPageCount", {
      sessionId: "sid",
      expectedContentGeneration: 1,
    }).catch((error) => error);

    expect(caught.code).toBe("PDFIUM_WORKER_CANCELLED");
    expect(caught.message).toBe(cause.message);
    expect(caught.userMessage).toBe(cause.message);
  });

  it("does not copy sensitive payload fields onto normalized errors", async () => {
    invokeMock.mockRejectedValueOnce("backend rejected authentication");

    const caught = await callBackend("authenticateDocumentSession", {
      sessionId: "sid",
      password: "secret-password",
    }).catch((error) => error);

    expect(caught).toMatchObject({
      command: "authenticate_document_session",
      message: "backend rejected authentication",
      cause: "backend rejected authentication",
    });
    expect(caught).not.toHaveProperty("payload");
    expect(caught).not.toHaveProperty("password");
    expect(JSON.stringify(caught)).not.toContain("secret-password");
  });

  it("profiles only registered commands through normal validation", async () => {
    await expect(
      invokeProfiled("unknown_command", {}),
    ).rejects.toMatchObject({
      command: "unknown_command",
      message: expect.stringContaining("Unknown backend command"),
    });
    expect(invokeMock).not.toHaveBeenCalled();

    await expect(
      invokeProfiled("merge_pdfs", {
        filePaths: "not-an-array",
        outputPath: "/tmp/out.pdf",
      }),
    ).rejects.toMatchObject({
      command: "merge_pdfs",
      message: expect.stringContaining("expected array"),
    });
    expect(invokeMock).not.toHaveBeenCalled();
  });
});

describe("binary render frame responses", () => {
  const pageRequest = {
    sessionId: "document",
    pageIndex: 1,
    scale: 2,
    rotation: 0,
    contentGeneration: 4,
    requestId: "main_page-request",
  };
  const thumbnailRequest = {
    sessionId: "document",
    pageIndex: 1,
    targetWidth: 1,
    contentGeneration: 4,
    requestId: "sidebar-request",
    scope: "sidebar_thumbnail",
  };

  it.each([
    ["renderSessionPage", pageRequest],
    ["renderSessionThumbnail", thumbnailRequest],
  ])("%s decodes the raw frame without a JSON pixel array", async (contractId, request) => {
    const buffer = encodeRenderFrame({
      width: 1,
      height: 1,
      data: new Uint8Array([10, 20, 30, 255]),
      pageIndex: request.pageIndex,
      generation: request.contentGeneration,
      requestId: request.requestId,
    });
    invokeMock.mockResolvedValueOnce(buffer);

    const result = await callBackend(contractId, request);

    expect(result).toMatchObject({
      width: 1,
      height: 1,
      pageIndex: 1,
      generation: 4,
      requestId: request.requestId,
    });
    expect(result.data).toBeInstanceOf(Uint8ClampedArray);
    expect(result.data.buffer).toBe(buffer);
  });

  it("profiled render calls do not log on success", async () => {
    const log = vi.spyOn(console, "log").mockImplementation(() => {});
    invokeMock.mockResolvedValueOnce(encodeRenderFrame({
      width: 1,
      height: 1,
      data: new Uint8Array([10, 20, 30, 255]),
      pageIndex: pageRequest.pageIndex,
      generation: pageRequest.contentGeneration,
      requestId: pageRequest.requestId,
    }));

    try {
      await expect(callBackendProfiled("renderSessionPage", pageRequest)).resolves.toMatchObject({
        width: 1,
        requestId: pageRequest.requestId,
      });
      expect(log).not.toHaveBeenCalled();
    } finally {
      log.mockRestore();
    }
  });

  it("reports a malformed frame through the existing invalid payload error", async () => {
    invokeMock.mockResolvedValueOnce(new ArrayBuffer(8));

    await expect(callBackend("renderSessionPage", pageRequest)).rejects.toMatchObject({
      name: "BackendCommandError",
      command: "render_session_page",
      message: "render_invalid_rgba_payload",
    });
  });

  it("keeps the JSON DocumentSessionError contract for failures", async () => {
    const backendError = {
      code: "PDFIUM_WORKER_RENDER_FAILED",
      message: "render failed",
    };
    invokeMock.mockRejectedValueOnce(backendError);

    await expect(callBackend("renderSessionThumbnail", thumbnailRequest)).rejects.toMatchObject({
      command: "render_session_thumbnail",
      code: "PDFIUM_WORKER_RENDER_FAILED",
      message: "render failed",
      cause: backendError,
    });
  });

  it("keeps render cancellation messages recognizable", async () => {
    invokeMock.mockRejectedValueOnce({
      code: "DOCUMENT_SESSION_PDF_ENGINE_ERROR",
      message: "render_cancelled",
    });

    await expect(callBackend("renderSessionPage", pageRequest)).rejects.toMatchObject({
      message: "render_cancelled",
    });
  });
});

describe("binary image frame responses", () => {
  const imageRequest = { path: "/a.png", rotation: 0, flipH: false, flipV: false };

  it("decodes read_image_file frames to width/height/pixels only", async () => {
    const buffer = encodeRenderFrame({
      width: 2,
      height: 1,
      data: new Uint8Array([1, 2, 3, 255, 4, 5, 6, 255]),
      pageIndex: 0,
      generation: 0,
      requestId: "read_image_file",
    });
    invokeMock.mockResolvedValueOnce(buffer);

    const result = await callBackend("readImageFile", imageRequest);

    expect(Object.keys(result)).toEqual(["width", "height", "data"]);
    expect(result).toMatchObject({ width: 2, height: 1 });
    expect(result.data).toBeInstanceOf(Uint8ClampedArray);
    expect(Array.from(result.data)).toEqual([1, 2, 3, 255, 4, 5, 6, 255]);
    expect(result.data.buffer).toBe(buffer);
  });

  it("rejects a legacy JSON pixel array response", async () => {
    invokeMock.mockResolvedValueOnce({ width: 1, height: 1, data: [1, 2, 3, 255] });

    await expect(callBackend("readImageFile", imageRequest)).rejects.toMatchObject({
      command: "read_image_file",
      message: "render_invalid_rgba_payload",
    });
  });

  it("keeps string errors for missing or corrupt images", async () => {
    invokeMock.mockRejectedValueOnce("Görsel okunamadı: Format error decoding Png");

    await expect(callBackend("readImageFile", imageRequest)).rejects.toMatchObject({
      name: "BackendCommandError",
      command: "read_image_file",
      code: null,
      message: "Görsel okunamadı: Format error decoding Png",
    });
  });
});
