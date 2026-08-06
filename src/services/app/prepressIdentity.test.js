import { describe, expect, it } from "vitest";
import { createPrepressContextKey, isCurrentPrepressEvent } from "./prepressIdentity.js";

describe("prepress identity", () => {
  const context = {
    isLoadedPdfDocument: true,
    sessionId: "session-A",
    contentGeneration: 4,
    physicalPageIndex: 2,
  };

  it("uses session, generation, and physical page instead of display path", () => {
    expect(createPrepressContextKey(context)).toBe("session-A\u00004\u00002");
    expect(createPrepressContextKey({ ...context, sessionId: "session-B" }))
      .not.toBe(createPrepressContextKey(context));
  });

  it("accepts only an event owned by the active prepress triplet", () => {
    expect(isCurrentPrepressEvent({ sessionId: "session-A", contentGeneration: 4, pageIndex: 2 }, context)).toBe(true);
    expect(isCurrentPrepressEvent({ sessionId: "session-A", contentGeneration: 5, pageIndex: 2 }, context)).toBe(false);
    expect(isCurrentPrepressEvent({ sessionId: "session-A", contentGeneration: 4, pageIndex: 3 }, context)).toBe(false);
  });
});
