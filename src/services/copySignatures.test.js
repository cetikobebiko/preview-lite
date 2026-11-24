import { describe, expect, it, vi } from "vitest";

vi.mock("./backend/session.js", () => ({ getSessionFormInfo: vi.fn() }));

import { copySignatureNote, readSignatureCount, withSignatureLoss } from "./copySignatures.js";

describe("copySignatures", () => {
  it("appends the loss sentence only when the backend report says so", () => {
    expect(withSignatureLoss("Kaydedildi.", { signaturesInvalidated: true }, "security")).toBe(
      "Kaydedildi. Belgedeki dijital imzalar kopyada geçersizdir.",
    );
    expect(withSignatureLoss("Bölündü.", { signaturesInvalidated: true }, "split")).toContain("bölünen dosyalarda");
    expect(withSignatureLoss("Birleşti.", { signaturesInvalidated: true }, "merge")).toContain("birleştirilen dosyada");
    expect(withSignatureLoss("Kaydedildi.", { signaturesInvalidated: false }, "security")).toBe("Kaydedildi.");
    expect(withSignatureLoss("Kaydedildi.", undefined, "security")).toBe("Kaydedildi.");
  });

  it("has a pre-run note for each copy kind that leaves the open document unaffected", () => {
    for (const kind of ["security", "split", "merge"]) {
      expect(copySignatureNote(kind)).toMatch(/^Bu belge dijital imza içeriyor\..*açık belgedeki imzalar etkilenmez\.$/i);
    }
  });

  it("reads the open document's signature count and treats failures as unknown", async () => {
    const lookup = vi.fn(async () => ({ form_type: "AcroForm", signature_count: 1 }));
    expect(await readSignatureCount("s1", 3, lookup)).toBe(1);
    expect(lookup).toHaveBeenCalledWith("s1", 3);
    expect(await readSignatureCount("s1", 3, async () => ({ form_type: "None", signature_count: 0 }))).toBe(0);
    expect(await readSignatureCount("s1", 3, async () => { throw new Error("stale"); })).toBe(0);
    expect(await readSignatureCount("", 3, lookup)).toBe(0);
    expect(lookup).toHaveBeenCalledTimes(1);
  });
});
