import { describe, expect, it, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import SignaturePresenceChip from "./SignaturePresenceChip.svelte";

async function settle() {
  for (let index = 0; index < 4; index += 1) {
    await Promise.resolve();
    await tick();
  }
}

describe("SignaturePresenceChip", () => {
  it("appears only when the document has signature fields and opens the panel", async () => {
    const onOpen = vi.fn();
    const lookup = vi.fn(async () => ({ signature_count: 1 }));
    const { findByTestId } = render(SignaturePresenceChip, {
      props: { sessionId: "s1", contentGeneration: 1, visible: true, onOpen, lookup },
    });
    await settle();
    await fireEvent.click(await findByTestId("signature-presence-chip"));
    expect(onOpen).toHaveBeenCalled();
    expect(lookup).toHaveBeenCalledWith("s1", 1);
  });

  it("stays hidden without fields, without visibility or when form info fails", async () => {
    for (const props of [
      { visible: true, lookup: async () => ({ signature_count: 0 }) },
      { visible: false, lookup: async () => ({ signature_count: 3 }) },
      { visible: true, lookup: async () => { throw new Error("form_editing off"); } },
    ]) {
      const { queryByTestId, unmount } = render(SignaturePresenceChip, {
        props: { sessionId: "s1", contentGeneration: 1, ...props },
      });
      await settle();
      expect(queryByTestId("signature-presence-chip")).toBeNull();
      unmount();
    }
  });
});
