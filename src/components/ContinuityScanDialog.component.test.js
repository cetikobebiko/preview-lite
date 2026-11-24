import { afterEach, describe, expect, it, vi } from "vitest";
import { flushSync, mount, unmount } from "svelte";
import ContinuityScanDialog from "./ContinuityScanDialog.svelte";

const hasClientMount = !String(mount).includes("lifecycle_function_unavailable");
const describeComponent = hasClientMount ? describe : describe.skip;

describeComponent("ContinuityScanDialog component behavior", () => {
  let component = null;

  afterEach(async () => {
    if (component) {
      await unmount(component);
      component = null;
    }
    document.body.replaceChildren();
  });

  function render() {
    const oncancel = vi.fn();
    component = mount(ContinuityScanDialog, { target: document.body, props: { oncancel } });
    flushSync();
    return { oncancel };
  }

  const cancelButton = () =>
    [...document.querySelectorAll("button")].find((button) => button.textContent.trim() === "İptal");

  it("shows the waiting state with the device requirements", () => {
    render();
    expect(document.body.textContent).toContain("iPhone/iPad'den bekleniyor");
    expect(document.body.textContent).toContain("Belgeleri Tara");
    expect(document.body.textContent).toContain("Bluetooth");
    expect(document.querySelector('[role="status"]')).not.toBeNull();
  });

  it("cancels once from the button and then shows that it is cancelling", () => {
    const { oncancel } = render();
    cancelButton().click();
    flushSync();
    cancelButton().click();
    flushSync();
    expect(oncancel).toHaveBeenCalledOnce();
    expect(cancelButton().disabled).toBe(true);
    expect(document.body.textContent).toContain("İptal ediliyor");
  });

  it("treats the close button as cancel", () => {
    const { oncancel } = render();
    document.querySelector('button[aria-label="Kapat"]').click();
    flushSync();
    expect(oncancel).toHaveBeenCalledOnce();
  });
});
