import { readFileSync, readdirSync } from "node:fs";
import { resolve, join } from "node:path";
import { describe, expect, it } from "vitest";

function readSource(relativePath) {
  return readFileSync(resolve(process.cwd(), relativePath), "utf8");
}

function getProdFrontendFiles(dirPath) {
  const results = [];
  const fullDirPath = resolve(process.cwd(), dirPath);
  const entries = readdirSync(fullDirPath, { withFileTypes: true });
  for (const entry of entries) {
    const fullPath = join(fullDirPath, entry.name);
    if (entry.isDirectory()) {
      if (["fixtures", "mocks", "__mocks__", "backend"].includes(entry.name)) continue;
      results.push(...getProdFrontendFiles(fullPath));
    } else if (entry.isFile()) {
      if (
        entry.name.endsWith(".test.js") ||
        entry.name.endsWith(".spec.js") ||
        entry.name === "actions.js" ||
        entry.name === "contracts.js"
      ) {
        continue;
      }
      if (entry.name.endsWith(".svelte") || entry.name.endsWith(".js")) {
        results.push(fullPath);
      }
    }
  }
  return results;
}

describe("UI feature wiring source contracts", () => {
  const appSource = readSource("src/App.svelte");
  const keyboardControllerSource = readSource("src/services/app/keyboardController.js");
  const viewerShellSource = readSource("src/components/ViewerShell.svelte");
  const toolbarNavigationSource = readSource("src/components/ToolbarNavigation.svelte");
  const markupToolbarSource = readSource("src/components/MarkupToolbar.svelte");
  const appToolbarSource = readSource("src/components/AppToolbar.svelte");
  const toolbarMenuSource = readSource("src/services/app/toolbarMenu.js");
  const documentExportControllerSource = readSource("src/services/app/documentExportController.js");
  const commandPaletteSource = readSource("src/components/CommandPalette.svelte");
  const pdfPageSource = readSource("src/components/PdfPage.svelte");
  const featureRuntimeSource = readSource("src/services/app/featureRuntime.js");
  const applicationRuntimeSource = readSource("src/services/app/applicationRuntime.js");
  const commandRouterSource = readSource("src/services/app/commandActionRouter.js");
  const appModalHostSource = readSource("src/components/AppModalHost.svelte");

  it("guards keyboard shortcuts and command selection before state changes", () => {
    expect(applicationRuntimeSource).toContain('featureRuntime.runUiAction("command_palette"');
    expect(applicationRuntimeSource).toContain('featureRuntime.runUiAction("search"');
    expect(applicationRuntimeSource).toContain("const keyboardController = createKeyboard({");
    expect(keyboardControllerSource).toContain('key === "s"');
    expect(keyboardControllerSource).toContain("event.shiftKey ? saveAs() : save()");
    expect(featureRuntimeSource).toContain("createCommandActionRouter({");
    expect(commandRouterSource).toContain("const command = getCommand(commandId)");
    expect(commandRouterSource).toContain("await guard(command.featureId");
    expect(commandPaletteSource).toContain("await selectCommand(command, featureContext");
    expect(commandPaletteSource).toContain("disabled={!cmd.presentation.runnable}");
  });

  it("gates annotation save controls separately from runnable annotation CRUD", () => {
    expect(appSource).toContain("createFeatureRuntime");
    expect(appSource).not.toContain("createDocumentExportController");
    // The annotation burn-in export was removed with its unregistered backend command.
    expect(documentExportControllerSource).not.toContain("annotation_burn_export");
    expect(appSource).toContain("<AppToolbar");
    expect(appSource).toContain("<MarkupToolbar");
    expect(appToolbarSource).toContain("{#if annotationVisible && isDirty}");
    expect(markupToolbarSource).toContain('featureUi("annotation")');
    expect(markupToolbarSource).toContain("badgeLabel(annotationState.badge)");
    expect(appToolbarSource).not.toContain("annotation_burn_export");
    expect(markupToolbarSource).not.toContain("annotation_burn_export");
    expect(appSource).toContain("onSave={documentRuntime.save}");
    expect(appToolbarSource).toContain("onclick={onSave}");
  });

  it("maps stop-speak to a dedicated stop-only handler without fire-and-forget", () => {
    expect(featureRuntimeSource).toContain('"stop-speak": tts.stopSpeaking');
    expect(appSource).toContain("onStopSpeaking={featureRuntime.stopSpeaking}");
    expect(featureRuntimeSource).not.toContain('"stop-speak": tts.speakCurrentPage');
  });

  it("keeps the basic viewer and zoom controls mounted independently of feature gates", () => {
    expect(appSource).toContain("<ViewerShell");
    expect(viewerShellSource).toContain("<PdfPage");
    expect(appToolbarSource).toContain("<ToolbarNavigation");
    expect(toolbarNavigationSource).not.toContain("featureUi");
    expect(toolbarNavigationSource).toContain("onclick={onZoomOut}");
    expect(toolbarNavigationSource).toContain("onclick={onResetZoom}");
    expect(toolbarNavigationSource).toContain("onclick={onZoomIn}");
    expect(pdfPageSource).toContain("<canvas");
  });

  it("gates nested mock and disabled modal mounts and their action handlers", () => {
    for (const source of [appToolbarSource, toolbarMenuSource]) {
      expect(source).not.toContain("pdf_x_export");
      expect(source).not.toContain("pades_signing");
      expect(source).not.toContain("signature_verification");
      expect(source).not.toContain("signature_settings");
    }

    expect(appModalHostSource).not.toContain("SignaturePanel");
    expect(appModalHostSource).not.toContain("SignatureWizardModal");
    expect(appModalHostSource).not.toContain("SignatureSettingsModal");

    expect(appModalHostSource).not.toContain("PdfXWizard");
    const sanitizationPanel = readSource("src/components/SanitizationPanel.svelte");
    expect(appModalHostSource).toContain('runnable={featureUi("sanitization").runnable}');
    expect(sanitizationPanel).toContain("disabled={!runnable");
    expect(sanitizationPanel).not.toContain("callBackend");
  });

  it("does not mount disabled page overlays from stale local UI state", () => {
    expect(pdfPageSource).toContain("{#if formPresentation.runnable}");
    expect(pdfPageSource).toContain("{#if annotationPresentation.runnable}");
    expect(pdfPageSource).not.toContain("redactionMaskPresentation");
  });

  it("enforces clean symbol invariant across production frontend source files", () => {
    const forbiddenSymbols = [
      "RedactionToolbar",
      "AutoRedactSidebar",
      "_redacted.pdf",
      "redactionMode",
      "redactionRects",
      "pageRedactionRects",
      "addRedaction",
      "onApplyRedactions",
      "onAddRedaction",
      "auto-redact",
      "autoRedact",
      "Otomatik Tarama",
      "karartma alanı",
      "applyRedactionsAction",
      "applyRedactions",
      "redaction-layer",
      "redaction-mark",
      "current-redaction",
    ];

    const prodFrontendFiles = [
      resolve(process.cwd(), "src/App.svelte"),
      ...getProdFrontendFiles("src/components"),
      ...getProdFrontendFiles("src/services/app"),
      ...getProdFrontendFiles("src/stores"),
    ];

    expect(prodFrontendFiles.length).toBeGreaterThan(10);

    for (const filePath of prodFrontendFiles) {
      const source = readFileSync(filePath, "utf8");
      for (const symbol of forbiddenSymbols) {
        expect(source, `${filePath} should not contain '${symbol}'`).not.toContain(symbol);
      }
    }
  });
});
