import { describe, expect, it } from "vitest";
import { get } from "svelte/store";
import {
  createPageFailureRegistry,
  pageFailureMessage,
} from "./pageFailureRegistry.js";

const page = { sessionId: "s1", contentGeneration: 2, pageIndex: 4 };
const failure = (code) => ({ code, message: "detail" });

describe("pageFailureRegistry", () => {
  it("tracks only worker-safety failures, per surface", () => {
    const registry = createPageFailureRegistry();
    expect(registry.record({ ...page, surface: "render" }, failure("PDFIUM_WORKER_CANCELLED"))).toBe(false);
    expect(registry.record({ ...page, surface: "render" }, new Error("render_timeout"))).toBe(false);
    expect(registry.record({ ...page, surface: "render" }, failure("PDFIUM_WORKER_RESTART_LIMIT"))).toBe(false);
    expect(registry.failure({ ...page, surface: "render" })).toBeNull();

    expect(registry.record({ ...page, surface: "render" }, failure("PDFIUM_WORKER_TIMEOUT"))).toBe(true);
    expect(registry.failure({ ...page, surface: "render" })).toBe("PDFIUM_WORKER_TIMEOUT");
    expect(registry.failure({ ...page, surface: "thumbnail" })).toBeNull();
    expect(registry.failure({ ...page, pageIndex: 5, surface: "render" })).toBeNull();
  });

  it("treats a memory-limit kill like a timeout and explains it", () => {
    const registry = createPageFailureRegistry();
    expect(registry.record({ ...page, surface: "render" }, failure("PDFIUM_WORKER_MEMORY_LIMIT"))).toBe(true);
    expect(registry.failure({ ...page, surface: "render" })).toBe("PDFIUM_WORKER_MEMORY_LIMIT");
    expect(pageFailureMessage("PDFIUM_WORKER_MEMORY_LIMIT", 4)).toBe(
      "Sayfa 5 işlenirken PDF motoru bellek sınırını aştığı için durduruldu.",
    );
  });

  it("blocks every page and surface of a quarantined document generation", () => {
    const registry = createPageFailureRegistry();
    registry.record({ ...page, surface: "thumbnail" }, failure("PDFIUM_DOCUMENT_QUARANTINED"));
    expect(registry.isDocumentQuarantined("s1", 2)).toBe(true);
    expect(registry.failure({ ...page, pageIndex: 0, surface: "textLayout" })).toBe(
      "PDFIUM_DOCUMENT_QUARANTINED",
    );
    // A page retry does not lift a document quarantine.
    registry.clearPage(page);
    expect(registry.isDocumentQuarantined("s1", 2)).toBe(true);
  });

  it("forgets a session's older generation and other sessions are independent", () => {
    const registry = createPageFailureRegistry();
    registry.record({ ...page, surface: "render" }, failure("PDFIUM_WORKER_TIMEOUT"));
    registry.record({ ...page, sessionId: "s2", surface: "render" }, failure("PDFIUM_WORKER_CRASHED"));
    registry.record({ ...page, contentGeneration: 3, surface: "render" }, failure("PDFIUM_PAGE_QUARANTINED"));

    expect(registry.failure({ ...page, surface: "render" })).toBeNull();
    expect(registry.failure({ ...page, contentGeneration: 3, surface: "render" })).toBe(
      "PDFIUM_PAGE_QUARANTINED",
    );
    expect(registry.failure({ ...page, sessionId: "s2", surface: "render" })).toBe(
      "PDFIUM_WORKER_CRASHED",
    );
  });

  it("clears selected surfaces of a page and notifies subscribers", () => {
    const registry = createPageFailureRegistry();
    registry.record({ ...page, surface: "render" }, failure("PDFIUM_WORKER_TIMEOUT"));
    registry.record({ ...page, surface: "textLayout" }, failure("PDFIUM_WORKER_TIMEOUT"));
    const before = get(registry);

    expect(registry.clearPage(page, ["render"])).toBe(true);
    expect(get(registry)).toBeGreaterThan(before);
    expect(registry.failure({ ...page, surface: "render" })).toBeNull();
    expect(registry.failure({ ...page, surface: "textLayout" })).toBe("PDFIUM_WORKER_TIMEOUT");

    expect(registry.clearPage(page)).toBe(true);
    expect(registry.failure({ ...page, surface: "textLayout" })).toBeNull();
    expect(registry.clearPage(page)).toBe(false);
  });

  it("keeps a bounded number of documents", () => {
    const registry = createPageFailureRegistry();
    for (let index = 0; index < 10; index += 1) {
      registry.record(
        { ...page, sessionId: `s${index}`, surface: "render" },
        failure("PDFIUM_WORKER_TIMEOUT"),
      );
    }
    expect(registry.failure({ ...page, sessionId: "s0", surface: "render" })).toBeNull();
    expect(registry.failure({ ...page, sessionId: "s9", surface: "render" })).toBe(
      "PDFIUM_WORKER_TIMEOUT",
    );
  });

  it("describes each tracked code in Turkish with a one-based page number", () => {
    expect(pageFailureMessage("PDFIUM_WORKER_TIMEOUT", 0)).toBe(
      "Sayfa 1 işlenirken zaman aşımı oluştu.",
    );
    expect(pageFailureMessage("PDFIUM_PAGE_QUARANTINED", 2)).toContain("Sayfa 3");
    expect(pageFailureMessage("PDFIUM_DOCUMENT_QUARANTINED", 2)).toContain("Belge");
    expect(pageFailureMessage("SOMETHING_ELSE", 0)).toBe("");
  });
});
