import { describe, expect, it } from "vitest";
import {
  parseMetricEvents,
  percentile,
  summarizeWorkerMetrics,
} from "./worker-metrics-report.mjs";

function line(seconds, fields, target = "pdfium_worker_metrics") {
  return JSON.stringify({
    timestamp: new Date(Date.UTC(2026, 9, 11, 12, 0, seconds)).toISOString(),
    level: "DEBUG",
    fields,
    target,
  });
}

describe("worker metrics report", () => {
  it("keeps only worker-metrics events and ignores malformed lines", () => {
    const text = [
      line(0, { event: "exchange", command_class: "render-page", outcome: "ok" }),
      line(1, { message: "unrelated" }, "preview_alternative_lib::document_session"),
      "not json",
      "",
    ].join("\n");
    const events = parseMetricEvents(text);
    expect(events).toHaveLength(1);
    expect(events[0].command_class).toBe("render-page");
  });

  it("computes nearest-rank percentiles", () => {
    expect(percentile([], 0.5)).toBeNull();
    expect(percentile([5, 1, 3, 2, 4], 0.5)).toBe(3);
    expect(percentile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 0.95)).toBe(10);
  });

  it("summarizes replacements, cancellation sources and exchange latency", () => {
    const events = parseMetricEvents(
      [
        line(0, { event: "worker_spawn", reason: "initial", spawn_to_hello_ms: 40 }),
        line(10, { event: "render_cancel", trigger: "explicit", reason: "viewport_exit", scope: "main_page" }),
        line(10, { event: "exchange", command_class: "render-page", outcome: "cancelled_killed", duration_ms: 500 }),
        line(10, { event: "worker_terminate", cause: "cancelled" }),
        line(11, { event: "worker_spawn", reason: "cancelled", spawn_to_hello_ms: 60 }),
        line(20, { event: "render_cancel", trigger: "slot_replaced", scope: "main_page" }),
        line(20, { event: "exchange", command_class: "render-page", outcome: "cancelled_drained", duration_ms: 90 }),
        line(30, { event: "exchange", command_class: "render-page", outcome: "ok", duration_ms: 80 }),
        line(31, { event: "exchange", command_class: "render-page", outcome: "ok", duration_ms: 120 }),
        line(40, { event: "admission", mode: "Read", outcome: "cancelled_in_queue", wait_ms: 3 }),
        line(41, { event: "admission", mode: "Read", outcome: "granted", wait_ms: 7 }),
        line(50, { event: "search_cancel", trigger: "search_superseded" }),
        line(60, { event: "search_completed", chunks: 4, duration_ms: 610 }),
      ].join("\n"),
    );
    const summary = summarizeWorkerMetrics(events);

    expect(summary.spanMinutes).toBe(1);
    expect(summary.workerSpawns).toEqual({ initial: 1, cancelled: 1 });
    expect(summary.replacementsPerMinute).toBe(1);
    expect(summary.terminations).toEqual({ cancelled: 1 });
    expect(summary.renderCancels).toEqual({
      "explicit:viewport_exit:main_page": 1,
      "slot_replaced:main_page": 1,
    });
    expect(summary.cancelObservedIn).toEqual({
      scheduler_queue: 0,
      admission_queue: 1,
      in_flight_drained: 1,
      in_flight_killed: 1,
    });
    expect(summary.exchangeMsByClass["render-page"]).toEqual({
      count: 2,
      p50: 80,
      p95: 120,
      p99: 120,
    });
    expect(summary.otherCancels).toEqual({ search_superseded: 1 });
    expect(summary.admissionWaitMs.Read.count).toBe(1);
    expect(summary.searches.count).toBe(1);
    expect(summary.searches.chunks.p50).toBe(4);
  });
});
