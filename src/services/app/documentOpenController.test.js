import { describe, expect, it } from "vitest";
import { isCurrentDocumentSnapshot } from "./documentOpenController.js";

describe("documentOpenController presentation guards", () => {
  it("requires both session identity and content generation", () => {
    const documentSession = { activeSessionId: "session-a", contentGeneration: 3 };
    expect(isCurrentDocumentSnapshot(documentSession, { id: "session-a", contentGeneration: 3 })).toBe(true);
    expect(isCurrentDocumentSnapshot(documentSession, { id: "session-a", contentGeneration: 2 })).toBe(false);
    expect(isCurrentDocumentSnapshot(documentSession, { id: "session-b", contentGeneration: 3 })).toBe(false);
  });
});
