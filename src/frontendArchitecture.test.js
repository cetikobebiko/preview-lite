import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path) => readFileSync(resolve(process.cwd(), path), "utf8");
const lines = (source) => source.split(/\r?\n/).length;
const appSource = read("src/App.svelte");

describe("frontend architecture acceptance gates", () => {
  it("keeps App below the hard size gates", () => {
    const script = appSource.match(/<script>([\s\S]*?)<\/script>/)?.[1] ?? "";
    expect(lines(appSource)).toBeLessThan(700);
    expect(lines(script)).toBeLessThan(200);
  });

  it("keeps every new composition and shell file below 600 lines", () => {
    for (const file of [
      "src/services/app/applicationRuntime.js",
      "src/services/app/documentRuntime.js",
      "src/services/app/viewerRuntime.js",
      "src/services/app/featureRuntime.js",
      "src/services/app/commandActionRouter.js",
      "src/services/app/presentationController.js",
      "src/components/AppHeader.svelte",
      "src/components/AppModalHost.svelte",
      "src/components/AppOverlayHost.svelte",
      "src/components/AppPresentationHost.svelte",
      "src/components/ViewerHudDock.svelte",
      "src/components/ViewerNavigationDock.svelte",
      "src/components/ViewerShell.svelte",
      "src/components/WorkspacePanels.svelte",
      "src/components/SearchHost.svelte",
    ]) expect(lines(read(file)), file).toBeLessThan(600);
  });

  it("keeps backend, platform and command orchestration outside App", () => {
    for (const forbidden of [
      "services/backend/",
      "services/actions.js",
      "configManager.js",
      "services/platform/dialogs.js",
      "services/platform/window.js",
      "getCommandById",
      "createDocumentExportController",
    ]) expect(appSource, forbidden).not.toContain(forbidden);
    expect(appSource).not.toMatch(/\bswitch\s*\(/);
    expect(appSource).not.toContain("onMount(async");
  });

  it("uses typed modal and panel APIs in the extracted shell", () => {
    const shell = [
      "AppHeader.svelte",
      "AppModalHost.svelte",
      "AppOverlayHost.svelte",
      "AppPresentationHost.svelte",
      "ViewerHudDock.svelte",
      "WorkspacePanels.svelte",
      "SearchHost.svelte",
    ].map((file) => read(`src/components/${file}`)).join("\n");
    for (const alias of [
      "showSidebar", "showOrganizer", "showMergeSplit", "showShortcuts",
      "isCmdPaletteOpen", "isSearchBarOpen", "isAiModalOpen", "isAiSettingsOpen",
      "isExportModalOpen", "isPdfXWizardOpen", "isSignatureWizardOpen",
      "isSignatureSettingsOpen", "isPluginManagerOpen", "isDeveloperConsoleOpen",
      "isA11yPanelOpen", "isPrepressPanelOpen", "isSignaturePanelOpen",
    ]) expect(shell, alias).not.toContain(`ui.${alias}`);
    expect(shell).toContain("ui.openModal(");
    expect(shell).toContain("ui.togglePanel(");
  });

  it("keeps export and password ownership in their bounded owners", () => {
    const documentRuntime = read("src/services/app/documentRuntime.js");
    const featureRuntime = read("src/services/app/featureRuntime.js");
    const documentLifecycle = read("src/services/app/documentLifecycleController.js");
    const presentation = read("src/services/app/presentationController.js");
    expect(documentRuntime).not.toMatch(/exportSessionPageToImage|extractSessionPages|convertAndSaveImage|burnAnnotationsToPdf|buildAnnotationMarkdown/);
    expect(featureRuntime).toMatch(/exportSessionPageToImage/);
    expect(featureRuntime).toMatch(/buildAnnotationMarkdown/);
    expect(documentLifecycle).toContain("let passwordAttempt = null");
    expect(documentLifecycle).toContain("passwordAttempt.attemptId !== attemptId");
    expect(presentation).not.toMatch(/let\s+attemptId|attemptId\s*\+\+|\+\+attemptId|attemptSequence/);
  });

  it("keeps the close transaction private to windowLifecycleController", () => {
    const windowLifecycle = read("src/services/app/windowLifecycleController.js");
    const documentRuntime = read("src/services/app/documentRuntime.js");
    expect(windowLifecycle).toContain("let closePromise = null");
    expect(windowLifecycle).toContain("event.preventDefault();");
    expect(documentRuntime).not.toContain("closePromise");
  });
});
