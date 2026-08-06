import { describe, expect, it } from "vitest";
import { createDocumentLifecycleController } from "./documentLifecycleController.js";

describe("document lifecycle controller", () => {
  it("keeps one password resolver and dismisses a superseded prompt", async () => {
    const prompts = [];
    const dismissals = [];
    const controller = createDocumentLifecycleController({
      documentSession: {}, applySnapshot: () => true, chooseSavePath: async () => null,
      askSaveConflict: async () => false, getDefaultFileName: () => "document.pdf",
      onPasswordPrompt: (payload) => prompts.push(payload),
      onPasswordDismiss: (payload) => dismissals.push(payload),
    });
    const first = controller.requestPassword({ attemptId: 1, path: "first.pdf" });
    const second = controller.requestPassword({ attemptId: 2, path: "second.pdf" });
    expect(await first).toBe(false);
    expect(prompts).toEqual([
      { attemptId: 1, candidateName: "first.pdf", error: "" },
      { attemptId: 2, candidateName: "second.pdf", error: "" },
    ]);
    expect(dismissals).toContainEqual({ attemptId: 1 });
    expect(controller.submitPassword("stale", 1)).toBe(false);
    expect(controller.submitPassword("secret", 2)).toBe(true);
    await expect(second).resolves.toBe("secret");
  });

  it("keeps authentication errors scoped to the authoritative attempt", async () => {
    const errors = [];
    const prompts = [];
    const controller = createDocumentLifecycleController({
      documentSession: {}, applySnapshot: () => true, chooseSavePath: async () => null,
      askSaveConflict: async () => false, getDefaultFileName: () => "document.pdf",
      onPasswordPrompt: (payload) => prompts.push(payload),
      onPasswordError: (payload) => errors.push(payload),
    });

    const firstPrompt = controller.requestPassword({ attemptId: 4, path: "/tmp/candidate.pdf" });
    expect(controller.submitPassword("wrong", 4)).toBe(true);
    await expect(firstPrompt).resolves.toBe("wrong");
    expect(controller.reportPasswordError({ attemptId: 3 })).toBe(false);
    expect(controller.reportPasswordError({ attemptId: 4 })).toBe(true);
    expect(errors).toEqual([{
      attemptId: 4,
      candidateName: "candidate.pdf",
      error: "Parola geçersiz. Yeniden deneyin.",
    }]);

    const retry = controller.requestPassword({ attemptId: 4, path: "/tmp/candidate.pdf" });
    expect(prompts.at(-1).error).toBe("Parola geçersiz. Yeniden deneyin.");
    expect(controller.cancelPassword(9)).toBe(false);
    expect(controller.cancelPassword(4)).toBe(true);
    await expect(retry).resolves.toBe(false);
  });

  it("does not invent a document identity when no session is active", async () => {
    const session = { activeSessionId: null, stateRevision: 4 };
    const controller = createDocumentLifecycleController({
      documentSession: session, applySnapshot: () => true, chooseSavePath: async () => null,
      askSaveConflict: async () => false, getDefaultFileName: () => "document.pdf",
    });
    const result = await controller.save();
    expect(result.status).toBe("unavailable");
  });
});
