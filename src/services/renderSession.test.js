import { describe, expect, it } from "vitest";
import {
  createRenderSession,
  requireActiveRenderSession,
} from "./renderSession.js";

describe("render session", () => {
  it("keeps one stable session object while replacing immutable snapshots", () => {
    const session = createRenderSession();
    const reference = session;
    const empty = session.snapshot();

    const active = session.activate("document-1", 1);

    expect(session).toBe(reference);
    expect(session.snapshot()).toBe(active);
    expect(active).toEqual({
      sessionId: "document-1",
      contentGeneration: 1,
    });
    expect(Object.isFrozen(active)).toBe(true);
    expect(empty).toEqual({ sessionId: null, contentGeneration: 0 });
  });

  it("takes a fresh immutable snapshot for each document generation", () => {
    const session = createRenderSession();
    const first = session.activate("document-1", 1);
    const second = session.activate("document-2", 2);

    expect(first).toEqual({
      sessionId: "document-1",
      contentGeneration: 1,
    });
    expect(second).toEqual({
      sessionId: "document-2",
      contentGeneration: 2,
    });
    expect(requireActiveRenderSession(session, "document-2", 2)).toBe(second);
    expect(() =>
      requireActiveRenderSession(session, "document-1", 1),
    ).toThrow("render_inactive_document");
  });

  it("clears the active identity without reassigning the session object", () => {
    const session = createRenderSession();
    session.activate("document-1", 1);

    const cleared = session.clear(2);

    expect(cleared).toEqual({
      sessionId: null,
      contentGeneration: 2,
    });
    expect(() =>
      requireActiveRenderSession(session, "document-1", 1),
    ).toThrow("render_inactive_document");
  });
});
