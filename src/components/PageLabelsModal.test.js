import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";

const sessionMocks = vi.hoisted(() => ({ getSessionPageLabels: vi.fn() }));

vi.mock("../services/backend/session.js", () => sessionMocks);
vi.mock("../pdfState.js", () => ({ setPageLabels: vi.fn() }));

import PageLabelsModal from "./PageLabelsModal.svelte";

describe("PageLabelsModal range preview", () => {
  const originalAnimate = Element.prototype.animate;

  beforeEach(() => {
    Element.prototype.animate = vi.fn(() => ({ finished: Promise.resolve(), cancel: vi.fn(), onfinish: null }));
    sessionMocks.getSessionPageLabels.mockResolvedValue([
      { startPage: 0, style: "lower_roman", prefix: "", firstNumber: 1 },
      { startPage: 1, style: "decimal", prefix: "", firstNumber: 1 },
    ]);
  });

  afterEach(() => {
    cleanup();
    Element.prototype.animate = originalAnimate;
  });

  it("does not preview a range past the last page when the next start is invalid", async () => {
    render(PageLabelsModal, { props: { sessionId: "s", contentGeneration: 1, pageCount: 3 } });
    expect(await screen.findByText("i")).toBeTruthy();
    expect(screen.getByText("1 … 2")).toBeTruthy();

    const secondStart = document.getElementById("label-start-1");
    await fireEvent.input(secondStart, { target: { value: "5" } });

    expect(await screen.findByText(/başlangıç sayfası 1 ile 3 arasında/)).toBeTruthy();
    expect(screen.getByText("i … iii")).toBeTruthy();
    expect(screen.queryByText("i … iv")).toBeNull();
  });
});
