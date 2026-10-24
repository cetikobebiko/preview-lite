import { beforeEach, describe, expect, it, vi } from "vitest";

const platform = vi.hoisted(() => ({
  readAppConfig: vi.fn(),
  writeAppConfig: vi.fn(),
}));
vi.mock("./services/backend/platform.js", () => platform);

// configManager keeps its cache at module level, so every test loads a fresh copy.
async function loadConfig(stored = null) {
  vi.resetModules();
  platform.readAppConfig.mockResolvedValue(stored === null ? null : JSON.stringify(stored));
  const config = await import("./configManager.js");
  await config.initConfig();
  return config;
}

const lastWritten = () => JSON.parse(platform.writeAppConfig.mock.calls.at(-1)[0]);

describe("configManager", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    platform.writeAppConfig.mockResolvedValue(undefined);
  });

  it("loads stored settings through the backend and writes no file on startup", async () => {
    const config = await loadConfig({ recentFiles: ["/a.pdf"], documentStates: { "/a.pdf": { page: 3 } } });

    expect(config.getConfigRecentFiles()).toEqual(["/a.pdf"]);
    expect(config.getDocumentState("/a.pdf")).toEqual({ page: 3 });
    expect(platform.writeAppConfig).not.toHaveBeenCalled();
  });

  it("starts from defaults when the backend has no config", async () => {
    const config = await loadConfig(null);

    expect(config.getConfigRecentFiles()).toEqual([]);
    expect(config.getDocumentState("/a.pdf")).toBeNull();
  });

  it("merges a document state and writes the whole config", async () => {
    const config = await loadConfig({ recentFiles: [], documentStates: { "/a.pdf": { page: 1, zoom: 1.5 } } });

    await config.saveDocumentState("/a.pdf", { page: 4 });

    expect(lastWritten()).toEqual({ recentFiles: [], documentStates: { "/a.pdf": { page: 4, zoom: 1.5 } } });
  });

  it("evicts the least recently updated document states beyond the limit", async () => {
    const { MAX_DOCUMENT_STATES } = await loadConfig(null);
    const states = Object.fromEntries(
      Array.from({ length: MAX_DOCUMENT_STATES }, (_, index) => [`/doc-${index}.pdf`, { page: index }]),
    );
    const config = await loadConfig({ recentFiles: [], documentStates: states });

    // Touching the oldest entry makes it the newest, so the next insert evicts doc-1 instead.
    await config.saveDocumentState("/doc-0.pdf", { page: 99 });
    await config.saveDocumentState("/new.pdf", { page: 1 });

    const written = Object.keys(lastWritten().documentStates);
    expect(written).toHaveLength(MAX_DOCUMENT_STATES);
    expect(written).not.toContain("/doc-1.pdf");
    expect(written.slice(-2)).toEqual(["/doc-0.pdf", "/new.pdf"]);
    expect(config.getDocumentState("/doc-0.pdf")).toEqual({ page: 99 });
  });

  it("trims an oversized stored map on load", async () => {
    const { MAX_DOCUMENT_STATES } = await loadConfig(null);
    const states = Object.fromEntries(
      Array.from({ length: MAX_DOCUMENT_STATES + 5 }, (_, index) => [`/doc-${index}.pdf`, { page: index }]),
    );
    const config = await loadConfig({ recentFiles: [], documentStates: states });

    expect(config.getDocumentState("/doc-4.pdf")).toBeNull();
    expect(config.getDocumentState("/doc-5.pdf")).toEqual({ page: 5 });
  });

  it("serializes overlapping saves so the last write carries the newest state", async () => {
    const config = await loadConfig(null);
    let releaseFirst;
    platform.writeAppConfig.mockImplementationOnce(() => new Promise((resolve) => { releaseFirst = resolve; }));

    const first = config.addRecentFile("/a.pdf");
    const second = config.saveDocumentState("/a.pdf", { page: 2 });
    await Promise.resolve();
    expect(platform.writeAppConfig).toHaveBeenCalledTimes(1);

    releaseFirst();
    await Promise.all([first, second]);

    expect(platform.writeAppConfig).toHaveBeenCalledTimes(2);
    expect(lastWritten()).toEqual({ recentFiles: ["/a.pdf"], documentStates: { "/a.pdf": { page: 2 } } });
  });

  it("keeps later saves working after a failed write", async () => {
    const config = await loadConfig(null);
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    platform.writeAppConfig.mockRejectedValueOnce(new Error("disk full"));

    await config.addRecentFile("/a.pdf");
    await config.addRecentFile("/b.pdf");

    expect(consoleError).toHaveBeenCalledWith("Config save error:", expect.any(Error));
    expect(lastWritten().recentFiles).toEqual(["/b.pdf", "/a.pdf"]);
    consoleError.mockRestore();
  });
});
