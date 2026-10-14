import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/svelte";
import OutlineItem from "./OutlineItem.svelte";

afterEach(cleanup);

const node = {
  title: "Bölüm 1",
  page_index: 4,
  children: [{ title: "Alt bölüm", page_index: 5, children: [] }],
};

describe("OutlineItem", () => {
  it("toggles children from the expand button without jumping", async () => {
    const onJump = vi.fn();
    const view = render(OutlineItem, { node, onJump });
    const toggle = view.getByRole("button", { name: "Genişlet" });

    await fireEvent.click(toggle);

    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(view.getByText("Alt bölüm")).toBeTruthy();
    expect(onJump).not.toHaveBeenCalled();
  });

  it("jumps to the entry page from a keyboard-focusable button", async () => {
    const onJump = vi.fn();
    const view = render(OutlineItem, { node, onJump });
    const entry = view.getByRole("button", { name: /Bölüm 1/ });

    await fireEvent.click(entry);

    expect(entry.tagName).toBe("BUTTON");
    expect(onJump).toHaveBeenCalledWith(4);
  });
});
