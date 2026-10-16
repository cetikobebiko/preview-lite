import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/svelte";
import ShortcutsModal from "./ShortcutsModal.svelte";

const originalAnimate = Element.prototype.animate;

beforeEach(() => {
  Element.prototype.animate = vi.fn(() => ({
    cancel: vi.fn(),
    finished: Promise.resolve(),
  }));
});

afterEach(() => {
  cleanup();
  Element.prototype.animate = originalAnimate;
});

describe("ShortcutsModal", () => {
  it("closes from the close button and Escape but not from clicks inside the dialog", async () => {
    const onclose = vi.fn();
    const view = render(ShortcutsModal, { onclose });
    const dialog = view.getByRole("dialog", { name: "Klavye Kısayolları" });

    await fireEvent.click(view.getByText("Dosya Aç"));
    await fireEvent.click(dialog);
    expect(onclose).not.toHaveBeenCalled();

    await fireEvent.click(view.getByRole("button", { name: "Kapat" }));
    expect(onclose).toHaveBeenCalledTimes(1);

    await fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    expect(onclose).toHaveBeenCalledTimes(2);
  });
});
