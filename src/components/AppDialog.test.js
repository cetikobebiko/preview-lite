import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { createRawSnippet, tick } from "svelte";
import AppDialog from "./AppDialog.svelte";

afterEach(() => cleanup());

const body = createRawSnippet(() => ({ render: () => `<p>Diyalog gövdesi</p>` }));

async function escape() {
  await fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
}

// Outside clicks go through bits-ui pointer tracking that jsdom does not drive;
// they are checked in the browser.
describe("AppDialog", () => {
  it("is a dialog labelled by its title that closes from Escape and the close button", async () => {
    const onclose = vi.fn();
    render(AppDialog, { props: { title: "Dışa Aktar", onclose, children: body } });
    await tick();

    const dialog = screen.getByRole("dialog", { name: "Dışa Aktar" });
    await fireEvent.click(screen.getByText("Diyalog gövdesi"));
    expect(onclose).not.toHaveBeenCalled();

    await fireEvent.click(screen.getByRole("button", { name: "Kapat" }));
    expect(onclose).toHaveBeenCalledTimes(1);

    await escape();
    await waitFor(() => expect(onclose).toHaveBeenCalledTimes(2));
    expect(dialog.isConnected).toBe(true);
  });

  it("must be answered explicitly when not dismissible", async () => {
    const onclose = vi.fn();
    render(AppDialog, { props: { title: "Parola", dismissible: false, onclose, children: body } });
    await tick();

    expect(screen.queryByRole("button", { name: "Kapat" })).toBeNull();
    await escape();
    await tick();
    expect(onclose).not.toHaveBeenCalled();
  });
});
