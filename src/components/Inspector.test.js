import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/svelte";
import { createRawSnippet, flushSync } from "svelte";
import Inspector from "./Inspector.svelte";

afterEach(() => cleanup());

const content = createRawSnippet((id) => ({ render: () => `<p>${id()} içeriği</p>` }));

describe("Inspector", () => {
  it("docks a single panel under its title without tabs", async () => {
    const onclose = vi.fn();
    const view = render(Inspector, { props: { panels: [{ id: "a11y", title: "Erişilebilirlik" }], onclose, content } });

    expect(view.getByRole("heading", { name: "Erişilebilirlik" })).toBeTruthy();
    expect(view.getByText("a11y içeriği")).toBeTruthy();
    expect(view.queryByRole("tablist")).toBeNull();
    await fireEvent.click(view.getByRole("button", { name: "Erişilebilirlik panelini kapat" }));
    expect(onclose).toHaveBeenCalledWith("a11y");
  });

  it("keeps every open panel mounted and brings a newly opened one to the front", async () => {
    const panels = [{ id: "a11y", title: "Erişilebilirlik" }];
    const view = render(Inspector, { props: { panels, onclose: vi.fn(), content } });

    await view.rerender({ panels: [...panels, { id: "prepress", title: "Baskı" }], onclose: vi.fn(), content });
    flushSync();
    expect(view.getByRole("tab", { name: "Baskı" }).getAttribute("aria-selected")).toBe("true");
    expect(view.getByText("a11y içeriği").closest("section").hidden).toBe(true);
    expect(view.getByText("prepress içeriği").closest("section").hidden).toBe(false);

    await fireEvent.click(view.getByRole("tab", { name: "Erişilebilirlik" }));
    expect(view.getByText("a11y içeriği").closest("section").hidden).toBe(false);
    expect(view.getByText("prepress içeriği").closest("section").hidden).toBe(true);
  });

  it("falls back to a remaining panel when the visible one closes", async () => {
    const both = [{ id: "a11y", title: "Erişilebilirlik" }, { id: "prepress", title: "Baskı" }];
    const view = render(Inspector, { props: { panels: both, onclose: vi.fn(), content } });

    await view.rerender({ panels: [both[0]], onclose: vi.fn(), content });
    flushSync();
    expect(view.getByText("a11y içeriği").closest("section").hidden).toBe(false);
  });
});
