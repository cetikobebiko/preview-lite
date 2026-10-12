import { afterEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { cleanup, fireEvent, render } from "@testing-library/svelte";
import BionicReadingModal from "./BionicReadingModal.svelte";
import { splitBionicWord, toBionicLines } from "../utils/bionicText.js";

afterEach(() => {
  cleanup();
});

function renderModal(props) {
  const result = render(BionicReadingModal, { props: { isOpen: true, ...props } });
  return { ...result, content: result.getByTestId("bionic-text") };
}

describe("BionicReadingModal XSS hardening", () => {
  it("does not use {@html}", () => {
    const source = readFileSync(resolve(process.cwd(), "src/components/BionicReadingModal.svelte"), "utf8");
    expect(source).not.toContain("@html");
  });

  it("renders an <img onerror> payload as literal text", () => {
    const payload = "<img src=x onerror=alert(1)>";
    const { container, content } = renderModal({ text: payload });

    expect(container.querySelector("img")).toBeNull();
    expect(content.textContent).toBe(payload);
  });

  it("renders a <script> payload as literal text", () => {
    const payload = "<script>alert(1)</script>";
    const { container, content } = renderModal({ text: payload });

    expect(container.querySelector("script")).toBeNull();
    expect(content.textContent).toBe(payload);
  });

  it("only creates <b> and <br> elements inside the text content", () => {
    const { content } = renderModal({ text: "<i>a</i> <svg onload=x>\n<iframe src=x>" });
    const tags = new Set(Array.from(content.querySelectorAll("*"), (el) => el.tagName));

    expect([...tags].sort()).toEqual(["B", "BR"]);
  });
});

describe("BionicReadingModal rendering", () => {
  it("bolds ceil(len * 0.4) characters and keeps the rest as plain text", () => {
    const { content } = renderModal({ text: "okuma hızı" });
    const bolds = Array.from(content.querySelectorAll("b"), (el) => el.textContent);

    expect(bolds).toEqual(["ok", "hı"]);
    expect(content.textContent).toBe("okuma hızı");
  });

  it("bolds single-character words completely", () => {
    const { content } = renderModal({ text: "a b" });
    expect(Array.from(content.querySelectorAll("b"), (el) => el.textContent)).toEqual(["a", "b"]);
  });

  it("renders each source line on its own line", () => {
    const { content } = renderModal({ text: "ilk satır\nikinci satır" });

    expect(content.querySelectorAll("br")).toHaveLength(1);
    expect(content.querySelectorAll("b")).toHaveLength(4);
  });

  it("shows the empty-text message", () => {
    const { content } = renderModal({ text: "" });

    expect(content.textContent.trim()).toBe("Metin yüklenmedi...");
    expect(content.querySelector("b")).toBeNull();
  });

  it("calls onclose from the close button and the overlay, but not the dialog body", async () => {
    const onclose = vi.fn();
    const { getByRole, container } = renderModal({ text: "metin", onclose });

    await fireEvent.click(container.querySelector(".bionic-dialog"));
    expect(onclose).not.toHaveBeenCalled();

    await fireEvent.click(getByRole("button", { name: "✕" }));
    expect(onclose).toHaveBeenCalledTimes(1);

    await fireEvent.click(container.querySelector(".bionic-overlay"));
    expect(onclose).toHaveBeenCalledTimes(2);
  });

  it("renders nothing while closed", () => {
    const { container } = render(BionicReadingModal, { props: { isOpen: false, text: "metin" } });
    expect(container.querySelector(".bionic-overlay")).toBeNull();
  });
});

describe("bionic word splitting", () => {
  it("uses ceil(len * 0.4) for the bold part", () => {
    expect(splitBionicWord("okuma")).toEqual({ bold: "ok", rest: "uma" });
    expect(splitBionicWord("ab")).toEqual({ bold: "a", rest: "b" });
    expect(splitBionicWord("x")).toEqual({ bold: "x", rest: "" });
  });

  it("never splits a surrogate pair", () => {
    // "😀" is two UTF-16 units; the old slice() split it into lone surrogates.
    expect(splitBionicWord("😀")).toEqual({ bold: "😀", rest: "" });
    const { bold, rest } = splitBionicWord("😀😀a");
    expect(bold).toBe("😀😀");
    expect(rest).toBe("a");
    expect(bold + rest).toBe("😀😀a");
  });

  it("keeps combining marks and ZWJ emoji with their base character", () => {
    const decomposedE = "é"; // é as base + combining acute
    expect(splitBionicWord(decomposedE)).toEqual({ bold: decomposedE, rest: "" });
    expect(splitBionicWord(`${decomposedE}${decomposedE}`)).toEqual({ bold: decomposedE, rest: decomposedE });

    const family = "👨‍👩‍👧";
    expect(splitBionicWord(`${family}ok`)).toEqual({ bold: `${family}o`, rest: "k" });
    expect(splitBionicWord(`${family}a`)).toEqual({ bold: family, rest: "a" });
  });

  it("returns no lines for empty input", () => {
    expect(toBionicLines("")).toEqual([]);
    expect(toBionicLines(null)).toEqual([]);
    expect(toBionicLines(undefined)).toEqual([]);
  });

  it("renders emoji and combining characters intact in the DOM", () => {
    const text = "été 😀mutlu";
    const { content } = renderModal({ text });

    expect(content.textContent).toBe(text);
    for (const node of content.querySelectorAll("b")) {
      expect(node.textContent).not.toMatch(/^[̀-ͯ]|[\uD800-\uDBFF]$/);
    }
  });

  it("labels the text as a single page and names its source", () => {
    const pdf = renderModal({ text: "Merhaba dünya", source: { pageNumber: 3, method: "pdf" } });
    expect(pdf.getByTestId("bionic-source").textContent).toBe("Yalnız sayfa 3 · PDF metin katmanı");
    cleanup();
    const ocr = renderModal({ text: "Merhaba", source: { pageNumber: 7, method: "ocr" } });
    expect(ocr.getByTestId("bionic-source").textContent).toBe("Yalnız sayfa 7 · OCR ile tanınan metin");
    cleanup();
    const none = renderModal({ text: "Merhaba" });
    expect(none.queryByTestId("bionic-source")).toBeNull();
  });
});
