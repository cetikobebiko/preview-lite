import { afterEach, describe, expect, it, vi } from "vitest";
import { flushSync, mount, unmount } from "svelte";
import AsyncPromptDialog from "./AsyncPromptDialog.svelte";

const hasClientMount = !String(mount).includes("lifecycle_function_unavailable");
const describeComponent = hasClientMount ? describe : describe.skip;

describeComponent("AsyncPromptDialog component behavior", () => {
  let component = null;

  afterEach(async () => {
    if (component) {
      await unmount(component);
      component = null;
    }
    document.body.replaceChildren();
  });

  // Regression: the input ref started as `undefined` while the shadcn Input's bindable `ref`
  // falls back to `null`, so Svelte threw props_invalid_value as soon as a prompt opened and the
  // UI stopped updating (Text note contents, "Sayfaya Git").
  it("opens an active request without a Svelte prop error and submits the typed value", () => {
    const coordinator = {
      active: { id: 1, title: "Not Açıklaması", label: "Notunuz", initialValue: "", placeholder: "" },
      submit: vi.fn(),
      cancel: vi.fn(),
    };

    expect(() => {
      component = mount(AsyncPromptDialog, { target: document.body, props: { coordinator } });
      flushSync();
    }).not.toThrow();

    const input = document.getElementById("async-prompt-input");
    expect(input).not.toBeNull();
    input.value = "Kaydedilmemiş not";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    document.getElementById("async-prompt-form").requestSubmit();

    expect(coordinator.submit).toHaveBeenCalledWith("Kaydedilmemiş not");
  });
});
