import { describe, expect, it, vi } from "vitest";
import {
  createSignatureLossGuard,
  isSignatureLossDeclined,
  signatureLossMessage,
  SignatureLossDeclinedError,
} from "./signatureLossGuard.js";

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));
vi.mock("./backend/session.js", () => ({ getSessionFormInfo: vi.fn() }));

function guard(signatureCount, answers = []) {
  const getFormInfo = vi.fn(async () => ({ form_type: "AcroForm", signature_count: signatureCount }));
  const confirm = vi.fn(async () => answers.shift() ?? false);
  return { ensure: createSignatureLossGuard({ getFormInfo, confirm }), getFormInfo, confirm };
}

describe("signature loss guard", () => {
  it("lets unsigned documents through without asking and checks once per session", async () => {
    const { ensure, getFormInfo, confirm } = guard(0);
    await ensure("s1", 1, "Sayfa döndürme");
    await ensure("s1", 2, "Sayfa silme");
    expect(getFormInfo).toHaveBeenCalledTimes(1);
    expect(getFormInfo).toHaveBeenCalledWith("s1", 1);
    expect(confirm).not.toHaveBeenCalled();
  });

  it("asks before rewriting a signed document, naming the operation", async () => {
    const { ensure, confirm } = guard(2, [true]);
    await ensure("s1", 3, "Açıklama ekleme");
    expect(confirm).toHaveBeenCalledWith(signatureLossMessage("Açıklama ekleme"));
    expect(signatureLossMessage("Açıklama ekleme")).toContain("imzaları geçersiz kılar");
    await ensure("s1", 4, "Sayfa silme");
    expect(confirm).toHaveBeenCalledTimes(1);
  });

  it("throws a recognizable cancellation when declined and asks again next time", async () => {
    const { ensure, confirm } = guard(1, [false, true]);
    const error = await ensure("s1", 1, "Sayfa silme").catch((caught) => caught);
    expect(error).toBeInstanceOf(SignatureLossDeclinedError);
    expect(isSignatureLossDeclined(error)).toBe(true);
    expect(isSignatureLossDeclined(new Error("other"))).toBe(false);
    await ensure("s1", 1, "Sayfa silme");
    expect(confirm).toHaveBeenCalledTimes(2);
  });

  it("propagates a failed signature lookup instead of guessing", async () => {
    const getFormInfo = vi.fn(async () => {
      throw new Error("worker unavailable");
    });
    const ensure = createSignatureLossGuard({ getFormInfo, confirm: vi.fn() });
    await expect(ensure("s1", 1, "Sayfa döndürme")).rejects.toThrow("worker unavailable");
  });
});
