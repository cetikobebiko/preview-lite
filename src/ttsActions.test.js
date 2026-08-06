import { beforeEach, describe, expect, it, vi } from "vitest";
import { FeatureUnavailableError } from "./featureActionGuard.js";
import { stopSpeaking } from "./ttsActions.js";

const backend = vi.hoisted(() => ({
  stopSpeaking: vi.fn(),
}));

vi.mock("./services/backend/platform.js", () => backend);

describe("text-to-speech stop action", () => {
  beforeEach(() => {
    backend.stopSpeaking.mockReset();
  });

  it("only invokes stop_speaking and updates stopped state", async () => {
    const onStopped = vi.fn();
    backend.stopSpeaking.mockResolvedValueOnce(undefined);

    await expect(
      stopSpeaking({ build: "production", platform: "macos" }, onStopped),
    ).resolves.toBeUndefined();

    expect(backend.stopSpeaking).toHaveBeenCalledOnce();
    expect(backend.stopSpeaking).toHaveBeenCalledWith();
    expect(onStopped).toHaveBeenCalledOnce();
  });

  it("does not invoke or mutate state when the platform is unavailable", async () => {
    const onStopped = vi.fn();

    await expect(
      stopSpeaking({ build: "development", platform: "linux" }, onStopped),
    ).rejects.toBeInstanceOf(FeatureUnavailableError);

    expect(backend.stopSpeaking).not.toHaveBeenCalled();
    expect(onStopped).not.toHaveBeenCalled();
  });
});
