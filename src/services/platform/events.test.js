import { describe, expect, it, vi } from "vitest";

const webviewOnDragDropEvent = vi.fn(async () => "unlisten");

vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: vi.fn(() => ({ onDragDropEvent: webviewOnDragDropEvent })),
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const { onDragDropEvent } = await import("./events.js");

describe("platform events", () => {
  it("subscribes to drag-drop through the current webview", async () => {
    const handler = vi.fn();

    await expect(onDragDropEvent(handler)).resolves.toBe("unlisten");

    expect(webviewOnDragDropEvent).toHaveBeenCalledExactlyOnceWith(handler);
  });
});
