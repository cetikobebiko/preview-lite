import { describe, expect, it, vi } from "vitest";
import { createDocumentTransformController } from "./documentTransformController.js";

function setup(overrides = {}) {
  const context = {
    sessionId: "session-a", contentGeneration: 3, path: "/tmp/original.pdf",
  };
  const guard = vi.fn(async (_feature, _runtime, action) => action());
  const merge = vi.fn(async () => undefined);
  const sanitize = vi.fn(async () => undefined);
  const openDocument = vi.fn(async () => undefined);
  const onToast = vi.fn();
  const controller = createDocumentTransformController({
    getContext: () => ({ ...context }), openFile: vi.fn(async () => "/tmp/second.pdf"),
    saveFile: vi.fn(async () => "/tmp/combined.pdf"), merge, sanitize,
    openDocument, guard, featureContext: {}, reportUnavailable: () => false,
    onToast, onError: vi.fn(), ...overrides,
  });
  return { controller, context, guard, merge, sanitize, openDocument, onToast };
}

describe("documentTransformController", () => {
  it("coordinates a guarded merge and opens its output only for the captured session", async () => {
    const { controller, guard, merge, openDocument } = setup();
    await controller.mergePdf();
    expect(guard).toHaveBeenCalledWith("page_merge_direct", {}, expect.any(Function));
    expect(merge).toHaveBeenCalledWith(["/tmp/original.pdf", "/tmp/second.pdf"], "/tmp/combined.pdf", {});
    expect(openDocument).toHaveBeenCalledWith("/tmp/combined.pdf");
  });

  it("does not expose a redact function", () => {
    const { controller } = setup();
    expect(controller.redact).toBeUndefined();
  });

  it("captures sanitization options under the feature guard", async () => {
    const { controller, sanitize, openDocument } = setup();
    await controller.sanitizeDocument({ removeMetadata: true });
    expect(sanitize).toHaveBeenCalledWith("session-a", 3, "/tmp/original_sanitized.pdf", { removeMetadata: true }, {});
    expect(openDocument).toHaveBeenCalledWith("/tmp/original_sanitized.pdf");
  });
});
