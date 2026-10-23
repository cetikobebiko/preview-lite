import { describe, expect, it } from "vitest";
import { phaseWindows } from "../src/devtools/cancelBaseline.js";
import { anchorOf, phaseVerdict, splitByPhase } from "./cancel-baseline-report.mjs";
import { summarizeWorkerMetrics } from "./worker-metrics-report.mjs";

const exchange = (at, command_class, outcome) => ({ at, event: "exchange", command_class, outcome, duration_ms: 5 });

describe("cancel-baseline-report", () => {
  it("lays phases out back to back with settle gaps", () => {
    expect(phaseWindows([{ phase: "a", durationMs: 10 }, { phase: "b", durationMs: 20 }], 5)).toEqual([
      ["a", 0, 10],
      ["b", 15, 35],
    ]);
  });

  it("anchors at the first successful page render and attributes events by window", () => {
    const windows = [["S1", 0, 100], ["S2", 150, 250]];
    const events = [
      exchange(900, "render-page", "cancelled_killed"),
      exchange(1000, "render-page", "ok"),
      exchange(1050, "render-page", "cancelled_drained"),
      exchange(1120, "render-thumbnail", "ok"), // settle gap
      exchange(1200, "render-page", "cancelled_killed"),
    ];
    const anchor = anchorOf(events);
    expect(anchor).toBe(1000);

    const phases = splitByPhase(events, anchor, windows);
    expect(phases.map((phase) => phase.events.length)).toEqual([2, 1]);
    expect(phaseVerdict(summarizeWorkerMetrics(phases[0].events))).toMatchObject({
      inFlightCancels: 1,
      drained: 1,
      killed: 0,
      drainRate: 1,
    });
    expect(phaseVerdict(summarizeWorkerMetrics(phases[1].events))).toMatchObject({ killed: 1, drainRate: 0 });
  });

  it("counts only replacement spawns", () => {
    const verdict = phaseVerdict(
      summarizeWorkerMetrics([
        { at: 1, event: "worker_spawn", reason: "initial", spawn_to_hello_ms: 3 },
        { at: 2, event: "worker_spawn", reason: "cancelled", spawn_to_hello_ms: 3 },
      ]),
    );
    expect(verdict.replacements).toBe(1);
  });
});
