import { describe, expect, it } from "vitest";
import { createFormDraftState } from "./formDraftState.svelte.js";

describe("form draft state", () => {
  it("keeps drafts per field name and clears them when the session changes", () => {
    const drafts = createFormDraftState();
    drafts.bindSession("session-a");
    drafts.set("name", { kind: "text", value: "Ada" });
    drafts.set("agree", { kind: "checkbox", checked: true });
    expect(drafts.count).toBe(2);
    expect(drafts.get("name")).toEqual({ kind: "text", value: "Ada" });

    drafts.bindSession("session-a");
    expect(drafts.count).toBe(2);

    drafts.bindSession("session-b");
    expect(drafts.count).toBe(0);
    expect(drafts.sessionId).toBe("session-b");
  });

  it("treats document field names as plain keys", () => {
    const drafts = createFormDraftState();
    expect(drafts.has("constructor")).toBe(false);
    expect(drafts.get("__proto__")).toBeUndefined();
    drafts.set("__proto__", { kind: "text", value: "x" });
    expect(drafts.get("__proto__")).toEqual({ kind: "text", value: "x" });
    drafts.discard("__proto__");
    expect(drafts.count).toBe(0);
  });

  it("returns a detached snapshot", () => {
    const drafts = createFormDraftState();
    drafts.set("langs", { kind: "choice", selected: [0, 2] });
    const snapshot = drafts.snapshot();
    snapshot[0][1].selected.push(9);
    expect(drafts.get("langs").selected).toEqual([0, 2]);
    drafts.discardAll();
    expect(drafts.count).toBe(0);
  });
});
