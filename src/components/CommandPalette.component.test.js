import { afterEach, describe, expect, it, vi } from "vitest";
import { flushSync, mount, unmount } from "svelte";
import CommandPalette from "./CommandPalette.svelte";

const productionMac = { build: "production", platform: "macos" };
const developmentMac = { build: "development", platform: "macos" };
const hasClientMount = !String(mount).includes("lifecycle_function_unavailable");
const describeComponent = hasClientMount ? describe : describe.skip;

describeComponent("CommandPalette component behavior", () => {
  let component = null;

  afterEach(async () => {
    if (component) {
      await unmount(component);
      component = null;
    }
    document.body.replaceChildren();
  });

  function renderPalette(context = productionMac) {
    const onselect = vi.fn();
    const onclose = vi.fn();
    component = mount(CommandPalette, {
      target: document.body,
      props: {
        isOpen: true,
        context,
        onselect,
        onclose,
      },
    });
    flushSync();
    return { onselect, onclose };
  }

  function filterBy(query) {
    const input = document.querySelector(".cmd-input");
    input.value = query;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
  }

  function pressKey(key) {
    window.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
    flushSync();
  }

  it("filters rendered commands", () => {
    renderPalette();
    filterBy("Apple Vision");

    const commands = [...document.querySelectorAll(".cmd-item")];
    expect(commands).toHaveLength(1);
    expect(commands[0].textContent).toContain("OCR");
  });

  it("moves selection with ArrowDown and ArrowUp", () => {
    renderPalette();
    const initial = document.querySelector(".cmd-item.active")?.textContent;

    pressKey("ArrowDown");
    const afterDown = document.querySelector(".cmd-item.active")?.textContent;
    expect(afterDown).not.toBe(initial);

    pressKey("ArrowUp");
    expect(document.querySelector(".cmd-item.active")?.textContent).toBe(initial);
  });

  it("emits the runnable command on Enter", async () => {
    const { onselect } = renderPalette();
    filterBy("Belge İçi Metin Arama");
    pressKey("Enter");

    await vi.waitFor(() => {
      expect(onselect).toHaveBeenCalledOnce();
    });
    expect(onselect.mock.calls[0][0].detail).toBe("open-search");
  });

  it("does not emit a disabled command on Enter", async () => {
    const { onselect } = renderPalette(developmentMac);
    filterBy("Markdown");

    expect(document.querySelector(".cmd-item")?.disabled).toBe(true);
    pressKey("Enter");
    await Promise.resolve();

    expect(onselect).not.toHaveBeenCalled();
    await vi.waitFor(() => {
      expect(document.querySelector(".cmd-unavailable-message")?.textContent).toContain(
        "gerçek işlem yapılmaz",
      );
    });
  });

  it("closes on Escape", async () => {
    const { onclose } = renderPalette();
    pressKey("Escape");

    await vi.waitFor(() => {
      expect(onclose).toHaveBeenCalledOnce();
    });
    expect(document.querySelector(".cmd-overlay")).toBeNull();
  });

  it("selects stop-speak without selecting the start command", async () => {
    const { onselect } = renderPalette();
    filterBy("Sesli Okumayı Durdur");
    pressKey("Enter");

    await vi.waitFor(() => {
      expect(onselect).toHaveBeenCalledOnce();
    });
    expect(onselect.mock.calls[0][0].detail).toBe("stop-speak");
    expect(onselect.mock.calls[0][0].detail).not.toBe("speak-page");
  });
});
