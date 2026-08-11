import { describe, expect, it } from "vitest";
import {
  createSecureRedactionState,
  DEFAULT_REDACTION_ACKNOWLEDGEMENTS,
  DEFAULT_SECURE_REDACTION_POLICY,
} from "./secureRedactionState.svelte.js";

const region = (id = "region-1") => ({
  id,
  pageIndex: 0,
  normalizedTopLeftRect: { x: 0.1, y: 0.2, width: 0.3, height: 0.2 },
  source: "manual_selection",
});
const readyPlan = () => ({
  digestVerified: true,
  executable: true,
  readyForExecution: true,
  blockers: [],
});

describe("secureRedactionState", () => {
  it("starts with conservative policy, separate lifecycles, and false acknowledgements", () => {
    const state = createSecureRedactionState();
    expect(state.status).toBe("idle");
    expect(state.pendingRegions).toEqual([]);
    expect(state.policy).toEqual(DEFAULT_SECURE_REDACTION_POLICY);
    expect(state.acknowledgements).toEqual(DEFAULT_REDACTION_ACKNOWLEDGEMENTS);
    expect(state.planning).toBe(false);
    expect(state.executing).toBe(false);
  });

  it("adds, removes, clears, and rejects duplicate region IDs", () => {
    const state = createSecureRedactionState();
    state.addRegion(region());
    expect(state.status).toBe("editing");
    expect(() => state.addRegion(region())).toThrow(/Duplicate region ID/);
    state.addRegion(region("region-2"));
    expect(state.removeRegion("region-1")).toBe(true);
    expect(state.pendingRegions.map(({ id }) => id)).toEqual(["region-2"]);
    state.clearRegions();
    expect(state.pendingRegions).toEqual([]);
  });

  it.each(["policy", "acknowledgement", "region"])("invalidates a ready plan after %s mutation", (kind) => {
    const state = createSecureRedactionState();
    state.addRegion(region());
    state.beginPlanning("plan-1", "session-1", 1);
    state.completePlanning("plan-1", readyPlan());
    expect(state.readyForExecution).toBe(true);
    if (kind === "policy") state.updatePolicy({ rasterDpi: 400 });
    if (kind === "acknowledgement") state.setAcknowledgement("rasterizedPagesAccepted", true);
    if (kind === "region") state.replaceRegion("region-1", { ...region(), normalizedTopLeftRect: { x: 0.2, y: 0.2, width: 0.3, height: 0.2 } });
    expect(state.plan).toBeNull();
    expect(state.result).toBeNull();
    expect(state.readyForExecution).toBe(false);
    expect(state.reviewPlan === null).toBe(kind !== "acknowledgement");
  });

  it("models planning, review, execution, success, failure, and reset independently", () => {
    const state = createSecureRedactionState();
    state.addRegion(region());
    state.beginPlanning("plan-1", "session-1", 7);
    expect(state.status).toBe("planning");
    state.completePlanning("plan-1", { ...readyPlan(), readyForExecution: false });
    expect(state.status).toBe("review_required");
    state.beginPlanning("plan-2", "session-1", 7);
    state.completePlanning("plan-2", readyPlan());
    expect(state.status).toBe("ready");
    state.beginExecution("exec-1", { planDigest: "a".repeat(64) });
    expect(state.status).toBe("executing");
    state.completeExecution("exec-1", { published: true });
    expect(state.status).toBe("completed");
    state.newRedaction();
    expect(state.status).toBe("idle");
    state.beginExecution("exec-2", {});
    state.failExecution("exec-2", { kind: "indeterminate", message: "unknown" });
    expect(state.status).toBe("failed");
    expect(state.executionError.kind).toBe("indeterminate");
    state.reset();
    expect(state.status).toBe("idle");
  });

  it("does not treat a merely present or blocked plan as ready", () => {
    const state = createSecureRedactionState();
    state.beginPlanning("plan-1", "session-1", 1);
    state.completePlanning("plan-1", { digestVerified: true, executable: true, readyForExecution: true, blockers: [{ code: "BLOCKED" }] });
    expect(state.readyForExecution).toBe(false);
  });
});
