import { beforeEach, describe, expect, it, vi } from "vitest";
import { callBackend } from "./client.js";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue({ id: "sid", contentGeneration: 2 });
});

const base = { sessionId: "sid", expectedContentGeneration: 1 };
const headerSpec = {
  texts: ["", "Rapor", "", "", "<<Sayfa 1 / n>>", ""],
  fontSize: 9,
  color: "#000000",
  margins: [36, 36, 72, 72],
  startNumber: 1,
  date: { year: 2026, month: 10, day: 28 },
};

describe("document structure contracts", () => {
  it("passes page mark ops in the shape `page_marks::PageMarkOp` reads", async () => {
    const op = { op: "header_footer", spec: headerSpec, replace: true };
    await callBackend("applySessionPageMarks", { ...base, op, pages: [] });
    expect(invokeMock).toHaveBeenCalledWith("apply_session_page_marks", { ...base, op, pages: [] });

    await callBackend("applySessionPageMarks", { ...base, op: { op: "remove", kinds: ["watermark"] }, pages: [0, 2] });
    expect(invokeMock).toHaveBeenLastCalledWith("apply_session_page_marks", {
      ...base,
      op: { op: "remove", kinds: ["watermark"] },
      pages: [0, 2],
    });
  });

  it("rejects a spec that belongs to another mark kind", async () => {
    await expect(
      callBackend("applySessionPageMarks", {
        ...base,
        op: { op: "watermark", spec: headerSpec, replace: false },
        pages: [],
      }),
    ).rejects.toMatchObject({ message: expect.stringContaining("watermark spec must carry exactly") });
    await expect(
      callBackend("applySessionPageMarks", { ...base, op: { op: "remove", kinds: [], replace: true }, pages: [] }),
    ).rejects.toMatchObject({ message: expect.stringContaining("remove op must carry exactly") });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("checks outline trees at every depth", async () => {
    const tree = [{ title: "A", source: null, pageIndex: 0, children: [{ title: "B", source: { path: [0, 0], title: "B" }, pageIndex: null, children: [] }] }];
    await callBackend("setSessionOutline", { ...base, tree });
    expect(invokeMock).toHaveBeenCalledWith("set_session_outline", { ...base, tree });

    const badChild = [{ ...tree[0], children: [{ title: "", children: [] }] }];
    await expect(callBackend("setSessionOutline", { ...base, tree: badChild })).rejects.toMatchObject({
      message: expect.stringContaining("tree[0].children[0].title"),
    });
  });

  it("checks page label ranges", async () => {
    const ranges = [{ startPage: 0, style: "lower_roman", prefix: "", firstNumber: 1 }];
    await callBackend("setSessionPageLabels", { ...base, ranges });
    expect(invokeMock).toHaveBeenCalledWith("set_session_page_labels", { ...base, ranges });
    await expect(
      callBackend("setSessionPageLabels", { ...base, ranges: [{ ...ranges[0], style: "roman" }] }),
    ).rejects.toMatchObject({ message: expect.stringContaining("style") });
  });

  it("sends ink strokes only when given", async () => {
    const rect = { x: 0.1, y: 0.1, width: 0.2, height: 0.2 };
    const annotation = { ...base, pageIndex: 0, annotType: "Ink", normalizedRect: rect, color: "#000000", contents: null };
    const inkStrokes = [[{ x: 0.1, y: 0.1 }, { x: 0.3, y: 0.3 }]];
    await callBackend("addSessionAnnotation", { ...annotation, inkStrokes });
    expect(invokeMock).toHaveBeenCalledWith("add_session_annotation", { ...annotation, inkStrokes });
  });
});
