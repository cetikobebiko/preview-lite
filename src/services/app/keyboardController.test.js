import { describe, expect, it, vi } from "vitest";
import { createKeyboardController } from "./keyboardController.js";

const actionNames = [
  "save", "saveAs", "toggleSidebar", "openFile", "toggleCommandPalette",
  "openSearch", "openExport", "requestPage", "goToPage", "adjustZoom",
  "toggleShortcuts", "nextPage", "previousPage", "undo", "redo",
];

function createController(overrides = {}) {
  const actions = Object.fromEntries(actionNames.map((name) => [name, vi.fn()]));
  const controller = createKeyboardController({
    hasDocument: () => true,
    isEditableTarget: () => false,
    ...actions,
    ...overrides,
  });
  return { controller, actions: { ...actions, ...overrides } };
}

function keyboardEvent(overrides = {}) {
  return {
    key: "",
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    preventDefault: vi.fn(),
    ...overrides,
  };
}

describe("keyboard controller behavior matrix", () => {
  it.each([
    ["Cmd+S", { metaKey: true, key: "s" }, "save", undefined],
    ["Ctrl+S", { ctrlKey: true, key: "S" }, "save", undefined],
    ["Cmd+Shift+S", { metaKey: true, shiftKey: true, key: "s" }, "saveAs", undefined],
    ["Ctrl+Shift+S", { ctrlKey: true, shiftKey: true, key: "S" }, "saveAs", undefined],
    ["Cmd/Ctrl+O", { ctrlKey: true, key: "o" }, "openFile", undefined],
    ["Cmd/Ctrl+B", { metaKey: true, key: "b" }, "toggleSidebar", undefined],
    ["Cmd/Ctrl+K", { ctrlKey: true, key: "k" }, "toggleCommandPalette", undefined],
    ["Cmd/Ctrl+F", { metaKey: true, key: "f" }, "openSearch", undefined],
    ["Cmd/Ctrl+P", { ctrlKey: true, key: "p" }, "openExport", undefined],
    ["Cmd/Ctrl+G", { metaKey: true, key: "g" }, "requestPage", undefined],
    ["zoom in", { ctrlKey: true, key: "=" }, "adjustZoom", 0.1],
    ["zoom out", { metaKey: true, key: "-" }, "adjustZoom", -0.1],
    ["zoom reset", { ctrlKey: true, key: "0" }, "adjustZoom", null],
    ["modifier shortcut help", { metaKey: true, key: "/" }, "toggleShortcuts", undefined],
    ["bare shortcut help", { key: "?" }, "toggleShortcuts", undefined],
    ["next ArrowRight", { key: "ArrowRight" }, "nextPage", undefined],
    ["next ArrowDown", { key: "ArrowDown" }, "nextPage", undefined],
    ["next PageDown", { key: "PageDown" }, "nextPage", undefined],
    ["previous ArrowLeft", { key: "ArrowLeft" }, "previousPage", undefined],
    ["previous ArrowUp", { key: "ArrowUp" }, "previousPage", undefined],
    ["previous PageUp", { key: "PageUp" }, "previousPage", undefined],
    ["Cmd+Z undo", { metaKey: true, key: "z" }, "undo", undefined],
    ["Ctrl+Z undo", { ctrlKey: true, key: "Z" }, "undo", undefined],
    ["Cmd+Shift+Z redo", { metaKey: true, shiftKey: true, key: "z" }, "redo", undefined],
    ["Ctrl+Y redo", { ctrlKey: true, key: "y" }, "redo", undefined],
  ])("routes %s and prevents the browser default exactly once", async (_name, keys, actionName, argument) => {
    const { controller, actions } = createController();
    const event = keyboardEvent(keys);

    const handled = await controller.handleKeyDown(event);

    expect(handled).not.toBe(false);
    if (argument === undefined) expect(actions[actionName]).toHaveBeenCalledWith();
    else expect(actions[actionName]).toHaveBeenCalledWith(argument);
    expect(actions[actionName]).toHaveBeenCalledOnce();
    expect(event.preventDefault).toHaveBeenCalledOnce();
  });

  it("leaves document-dependent shortcuts untouched when no document is open", async () => {
    const { controller, actions } = createController({ hasDocument: () => false });
    const event = keyboardEvent({ ctrlKey: true, key: "f" });

    await expect(controller.handleKeyDown(event)).resolves.toBe(false);
    expect(actions.openSearch).not.toHaveBeenCalled();
    expect(event.preventDefault).not.toHaveBeenCalled();
  });

  it.each(["input", "textarea", "contenteditable"])(
    "does not navigate or prevent default while editing a %s target",
    async (targetType) => {
      const element = document.createElement(targetType === "contenteditable" ? "div" : targetType);
      if (targetType === "contenteditable") {
        element.contentEditable = "true";
        Object.defineProperty(element, "isContentEditable", { value: true });
        element.tabIndex = 0;
      }
      document.body.append(element);
      element.focus();
      const isEditableTarget = () => {
        const active = document.activeElement;
        const tag = active?.tagName?.toLowerCase();
        return tag === "input" || tag === "textarea" || active?.isContentEditable;
      };
      const { controller, actions } = createController({ isEditableTarget });
      const event = keyboardEvent({ key: "ArrowRight" });

      await expect(controller.handleKeyDown(event)).resolves.toBe(false);
      expect(actions.nextPage).not.toHaveBeenCalled();
      expect(event.preventDefault).not.toHaveBeenCalled();
      element.remove();
    },
  );

  it("does not prevent default for an unknown key", async () => {
    const { controller } = createController();
    const event = keyboardEvent({ key: "x" });

    await expect(controller.handleKeyDown(event)).resolves.toBe(false);
    expect(event.preventDefault).not.toHaveBeenCalled();
  });
});

describe("document undo and redo shortcuts", () => {
  it("leave Cmd+Z to the browser while a text field is focused", async () => {
    const { controller, actions } = createController({ isEditableTarget: () => true });
    for (const keys of [{ metaKey: true, key: "z" }, { metaKey: true, shiftKey: true, key: "z" }, { ctrlKey: true, key: "y" }]) {
      const event = keyboardEvent(keys);
      expect(await controller.handleKeyDown(event)).toBe(false);
      expect(event.preventDefault).not.toHaveBeenCalled();
    }
    expect(actions.undo).not.toHaveBeenCalled();
    expect(actions.redo).not.toHaveBeenCalled();
  });

  it("do nothing without an open document", async () => {
    const { controller, actions } = createController({ hasDocument: () => false });
    const event = keyboardEvent({ metaKey: true, key: "z" });
    expect(await controller.handleKeyDown(event)).toBe(false);
    expect(actions.undo).not.toHaveBeenCalled();
  });
});
