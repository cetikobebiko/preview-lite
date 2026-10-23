import { readdirSync, readFileSync } from "node:fs";
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
      "src/components/AppToolbar.svelte",
      "src/components/MarkupToolbar.svelte",
      "src/components/AppModalHost.svelte",
      "src/components/AppOverlayHost.svelte",
      "src/components/AppPresentationHost.svelte",
      "src/components/ToolbarNavigation.svelte",
      "src/services/app/toolbarMenu.js",
      "src/components/ViewerShell.svelte",
      "src/components/WorkspacePanels.svelte",
      "src/components/SearchHost.svelte",
      "src/components/SecureRedactionPanel.svelte",
      "src/services/app/secureRedactionController.js",
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

  it("grants the webview no filesystem or shell access", () => {
    // File writes go through Rust (document_io::write_atomically); the webview only opens dialogs.
    const capability = JSON.parse(read("src-tauri/capabilities/default.json"));
    const pkg = JSON.parse(read("package.json"));
    const lib = read("src-tauri/src/lib.rs");
    for (const prefix of ["fs:", "shell:"]) {
      expect(capability.permissions.filter((permission) => permission.startsWith(prefix))).toEqual([]);
    }
    expect(Object.keys(pkg.dependencies)).not.toContain("@tauri-apps/plugin-fs");
    expect(Object.keys(pkg.dependencies)).not.toContain("@tauri-apps/plugin-shell");
    expect(lib).not.toMatch(/tauri_plugin_(fs|shell)::init/);
  });

  it("keeps every Svelte component on runes syntax (AGENTS.md)", () => {
    const components = readdirSync(resolve(process.cwd(), "src"), { recursive: true })
      .filter((file) => file.endsWith(".svelte"));
    expect(components.length).toBeGreaterThan(0);
    for (const file of components) {
      const source = read(`src/${file}`);
      expect(source, file).not.toMatch(/^\s*\$:/m);
      expect(source, file).not.toMatch(/\bexport let\b/);
      expect(source, file).not.toMatch(/\son:[a-z]+(\||=|\s|\/?>)/);
      expect(source, file).not.toContain("createEventDispatcher");
    }
  });

  it("keeps secure-redaction business logic in its canonical controller and behind panel/runtime gates", () => {
    const controller = read("src/services/app/secureRedactionController.js");
    const workspace = read("src/components/WorkspacePanels.svelte");
    const viewerShell = read("src/components/ViewerShell.svelte");
    expect(controller).toContain("startSecureRedactionPlan");
    expect(controller).toContain("startSecureRedactionExecution");
    expect(appSource).not.toContain("startSecureRedactionPlan");
    expect(appSource).not.toContain("startSecureRedactionExecution");
    expect(appSource).not.toContain("planDigest");
    expect(workspace).toContain('featureUi("redaction").visible');
    expect(viewerShell).toContain('featureUi("redaction").runnable');
    for (const file of [
      "src/App.svelte",
      "src/components/PdfPage.svelte",
      "src/components/SensitiveDataScanPanel.svelte",
      "src/components/SecureRedactionPanel.svelte",
    ]) {
      const source = read(file);
      expect(source, file).not.toContain("startSecureRedactionPlan");
      expect(source, file).not.toContain("startSecureRedactionExecution");
      expect(source, file).not.toContain('invoke("plan_secure_redaction"');
      expect(source, file).not.toContain('invoke("execute_secure_redaction"');
    }
  });

  it("uses typed modal and panel APIs in the extracted shell", () => {
    const shell = [
      "components/AppToolbar.svelte",
      "components/AppModalHost.svelte",
      "components/AppOverlayHost.svelte",
      "components/AppPresentationHost.svelte",
      "components/MarkupToolbar.svelte",
      "components/WorkspacePanels.svelte",
      "components/SearchHost.svelte",
      "services/app/toolbarMenu.js",
    ].map((file) => read(`src/${file}`)).join("\n");
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
