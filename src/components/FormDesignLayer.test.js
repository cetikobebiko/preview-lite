import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/svelte";
import { getPageFormFields } from "../pdfState.js";
import { createFormDesignState, FORM_DESIGN_CONTEXT } from "../stores/formDesignState.svelte.js";
import FormDesignLayer from "./FormDesignLayer.svelte";

vi.mock("../pdfState.js", () => ({ getPageFormFields: vi.fn(), authorFormFields: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));
vi.mock("../services/backend/session.js", () => ({ getSessionFormInfo: vi.fn() }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function widget(annot_index, field_name, kind, normalized_rect) {
  return { annot_index, field_name, kind, normalized_rect, hidden: false, read_only: false, required: false, multiline: false, options: [] };
}

const WIDGETS = [
  widget(0, "name", "Text", { x: 0.1, y: 0.1, width: 0.4, height: 0.05 }),
  widget(1, "Group1", "RadioButton", { x: 0.1, y: 0.3, width: 0.05, height: 0.03 }),
];

// 600×800 layer, no rotation, drawn at 1:1 at the origin.
function stubLayer(layer) {
  Object.defineProperty(layer, "offsetWidth", { configurable: true, value: 600 });
  Object.defineProperty(layer, "offsetHeight", { configurable: true, value: 800 });
  layer.getBoundingClientRect = () => ({ left: 0, top: 0, width: 600, height: 800, right: 600, bottom: 800, x: 0, y: 0 });
  layer.setPointerCapture = () => {};
  layer.releasePointerCapture = () => {};
}

function setup(props = {}, widgets = WIDGETS) {
  getPageFormFields.mockResolvedValue(widgets);
  const author = vi.fn(async () => ({ contentGeneration: 6 }));
  const design = createFormDesignState({ author, ensureSigned: vi.fn(async () => {}) });
  const onsessionSnapshot = vi.fn();
  const view = render(FormDesignLayer, {
    props: { sessionId: "s", contentGeneration: 5, pageIndex: 0, pageWidth: 600, pageHeight: 800, onsessionSnapshot, ...props },
    context: new Map([[FORM_DESIGN_CONTEXT, design]]),
  });
  const layer = view.container.querySelector(".form-design-layer");
  stubLayer(layer);
  return { ...view, layer, design, author, onsessionSnapshot };
}

async function findBox(container, annotIndex) {
  return waitFor(() => {
    const box = container.querySelector(`[data-annot-index="${annotIndex}"]`);
    if (!box) throw new Error(`widget ${annotIndex} not rendered`);
    return box;
  });
}

const at = (x, y) => ({ button: 0, pointerId: 1, clientX: x * 600, clientY: y * 800 });

describe("FormDesignLayer", () => {
  it("adds a field of the active kind in the dragged rect", async () => {
    const { layer, author, onsessionSnapshot } = setup({ activeTool: "form-text" });
    await waitFor(() => expect(getPageFormFields).toHaveBeenCalledWith("s", 5, 0));
    await fireEvent.pointerDown(layer, at(0.5, 0.5));
    await fireEvent.pointerMove(layer, at(0.8, 0.55));
    await fireEvent.pointerUp(layer, at(0.8, 0.55));
    await waitFor(() => expect(author).toHaveBeenCalledTimes(1));
    const [session, generation, [op]] = author.mock.calls[0];
    expect([session, generation]).toEqual(["s", 5]);
    expect(op).toMatchObject({ op: "add", pageIndex: 0, field: { kind: "text", multiline: false }, name: null });
    expect(op.normalizedRect.x).toBeCloseTo(0.5, 9);
    expect(op.normalizedRect.width).toBeCloseTo(0.3, 9);
    expect(op.normalizedRect.height).toBeCloseTo(0.05, 9);
    expect(onsessionSnapshot).toHaveBeenCalledWith({ contentGeneration: 6 });
  });

  it("adds a default-sized field on a click and picks a fresh radio group", async () => {
    const { layer, author } = setup({ activeTool: "form-radio" });
    await waitFor(() => expect(getPageFormFields).toHaveBeenCalled());
    await new Promise((resolve) => setTimeout(resolve, 0));
    await fireEvent.pointerDown(layer, at(0.5, 0.5));
    await fireEvent.pointerUp(layer, at(0.5, 0.5));
    await waitFor(() => expect(author).toHaveBeenCalledTimes(1));
    const [op] = author.mock.calls[0][2];
    expect(op.field).toEqual({ kind: "radio", group: "Group2", export: "Choice1" });
    expect(op.normalizedRect.width).toBeCloseTo(14 / 600, 9);
    expect(op.normalizedRect.height).toBeCloseTo(14 / 800, 9);
  });

  it("selects, moves and resizes a field and deletes it with the Delete key", async () => {
    const { container, layer, author, design } = setup({ activeTool: "form-select" });
    const box = await findBox(container, 0);
    await fireEvent.pointerDown(box, at(0.2, 0.12));
    expect(design.selection.widget.field_name).toBe("name");
    await fireEvent.pointerMove(layer, at(0.3, 0.22));
    await fireEvent.pointerUp(layer, at(0.3, 0.22));
    await waitFor(() => expect(author).toHaveBeenCalledTimes(1));
    let [op] = author.mock.calls[0][2];
    expect(op).toMatchObject({ op: "move", annotIndex: 0, fieldName: "name" });
    expect(op.normalizedRect.x).toBeCloseTo(0.2, 9);
    expect(op.normalizedRect.y).toBeCloseTo(0.2, 9);
    expect(op.normalizedRect.width).toBeCloseTo(0.4, 9);

    await fireEvent.pointerDown(box, at(0.2, 0.12));
    const handle = container.querySelector('[data-handle="resize"]');
    await fireEvent.pointerDown(handle, at(0.5, 0.15));
    await fireEvent.pointerMove(layer, at(0.6, 0.2));
    await fireEvent.pointerUp(layer, at(0.6, 0.2));
    await waitFor(() => expect(author).toHaveBeenCalledTimes(2));
    [op] = author.mock.calls[1][2];
    expect(op.op).toBe("move");
    expect(op.normalizedRect.x).toBeCloseTo(0.1, 9);
    expect(op.normalizedRect.width).toBeCloseTo(0.5, 9);
    expect(op.normalizedRect.height).toBeCloseTo(0.1, 9);

    await fireEvent.pointerDown(box, at(0.2, 0.12));
    await fireEvent.pointerUp(layer, at(0.2, 0.12));
    expect(author).toHaveBeenCalledTimes(2);
    await fireEvent.keyDown(window, { key: "Delete" });
    await waitFor(() => expect(author).toHaveBeenCalledTimes(3));
    expect(author.mock.calls[2][2]).toEqual([{ op: "delete", pageIndex: 0, annotIndex: 0, fieldName: "name" }]);
  });

  it("selects the newly added widget once the page reloads", async () => {
    const { layer, design, rerender } = setup({ activeTool: "form-checkbox" });
    await waitFor(() => expect(getPageFormFields).toHaveBeenCalledTimes(1));
    await fireEvent.pointerDown(layer, at(0.6, 0.6));
    await fireEvent.pointerUp(layer, at(0.6, 0.6));
    await waitFor(() => expect(design.pendingSelect).not.toBeNull());
    getPageFormFields.mockResolvedValue([
      ...WIDGETS,
      widget(2, "Check Box1", "Checkbox", { x: 0.6, y: 0.6, width: 0.02, height: 0.02 }),
    ]);
    await rerender({ contentGeneration: 6 });
    await waitFor(() => expect(design.selection?.widget.field_name).toBe("Check Box1"));
    expect(design.selection.contentGeneration).toBe(6);
    expect(design.pendingSelect).toBeNull();
  });

  it("clicking empty space with the select tool clears the selection", async () => {
    const { container, layer, design } = setup({ activeTool: "form-select" });
    const box = await findBox(container, 0);
    await fireEvent.pointerDown(box, at(0.2, 0.12));
    await fireEvent.pointerUp(layer, at(0.2, 0.12));
    expect(design.selection).not.toBeNull();
    await fireEvent.pointerDown(layer, at(0.9, 0.9));
    expect(design.selection).toBeNull();
  });
});
