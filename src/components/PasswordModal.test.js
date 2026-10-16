import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import PasswordModal from "./PasswordModal.svelte";

afterEach(() => cleanup());

function renderModal(props = {}) {
  const onsubmit = vi.fn();
  const oncancel = vi.fn();
  render(PasswordModal, { props: { isOpen: true, fileName: "gizli.pdf", onsubmit, oncancel, ...props } });
  return { onsubmit, oncancel, input: screen.getByLabelText("Parola") };
}

describe("PasswordModal", () => {
  it("submits with Enter and clears the password from state", async () => {
    const { onsubmit, input } = renderModal();

    await fireEvent.input(input, { target: { value: "s3cret" } });
    await fireEvent.keyDown(input, { key: "Enter" });

    expect(onsubmit).toHaveBeenCalledWith("s3cret");
    await tick();
    expect(input.value).toBe("");
  });

  it("cancels once from Escape in the field and offers no dismiss button", async () => {
    const { oncancel, input } = renderModal();

    expect(screen.getByRole("dialog", { name: "Parola gerekli" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Kapat" })).toBeNull();
    await fireEvent.keyDown(input, { key: "Escape" });
    await tick();
    expect(oncancel).toHaveBeenCalledTimes(1);
  });

  it("keeps unlock disabled until a password is typed and shows the error", async () => {
    renderModal({ errorMsg: "Parola hatalı" });

    expect(screen.getByRole("button", { name: "Kilidi Aç" }).disabled).toBe(true);
    expect(screen.getByRole("alert").textContent).toBe("Parola hatalı");
    expect(screen.getByLabelText("Parola").getAttribute("aria-invalid")).toBe("true");
  });
});
