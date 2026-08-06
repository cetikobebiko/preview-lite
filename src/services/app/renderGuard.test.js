import { describe, expect, it } from "vitest";
import { evaluateRenderGuard } from "./renderGuard.js";

const pdf = (overrides = {}) => ({
  isMounted: true,
  request: 4,
  activeRequest: 4,
  isPdf: true,
  capturedSessionId: "session-A",
  currentSessionId: "session-A",
  capturedGeneration: 3,
  currentGeneration: 3,
  capturedImagePath: null,
  currentImagePath: null,
  ...overrides,
});

describe("evaluateRenderGuard", () => {
  it("rejects a PDF result after the active session changes", () => {
    expect(evaluateRenderGuard(pdf({ currentSessionId: "session-B" }))).toBe(false);
  });

  it("rejects a PDF result after content generation changes", () => {
    expect(evaluateRenderGuard(pdf({ currentGeneration: 4 }))).toBe(false);
  });

  it("rejects a PDF result after switching to an image", () => {
    expect(evaluateRenderGuard(pdf({ isPdf: false, currentImagePath: "/tmp/image.png" }))).toBe(false);
  });

  it("rejects image A after image B becomes active", () => {
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
        capturedImagePath: "/tmp/a.png",
        currentImagePath: "/tmp/b.png",
      }),
    ).toBe(false);
  });

  it("rejects unmounted and superseded requests", () => {
    expect(evaluateRenderGuard(pdf({ isMounted: false }))).toBe(false);
    expect(evaluateRenderGuard(pdf({ activeRequest: 5 }))).toBe(false);
  });
});
