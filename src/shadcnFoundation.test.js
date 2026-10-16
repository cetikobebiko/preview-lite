import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { render, screen } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import { Button } from "$lib/components/ui/button";
import { Toggle } from "$lib/components/ui/toggle";

const appCss = readFileSync(resolve(process.cwd(), "src/app.css"), "utf8");

describe("shadcn-svelte foundation", () => {
  it("resolves $lib components and renders them with Tailwind classes", () => {
    render(Button, { props: { variant: "ghost", size: "icon", "aria-label": "Yakınlaştır" } });
    const button = screen.getByRole("button", { name: "Yakınlaştır" });
    expect(button.dataset.slot).toBe("button");
    expect(button.className).toContain("rounded-md");

    render(Toggle, { props: { pressed: true, "aria-label": "Kenar çubuğu" } });
    expect(screen.getByRole("button", { name: "Kenar çubuğu" }).getAttribute("aria-pressed")).toBe("true");
  });

  it("imports $lib modules extensionless from JavaScript sources", () => {
    // Vite maps `.js` to `.ts` only for TypeScript importers, so a plain
    // <script> importing "$lib/.../index.js" builds in Vitest but not in Vite.
    const offenders = readdirSync(resolve(process.cwd(), "src"), { recursive: true })
      .filter((file) => /\.(svelte|js)$/.test(file) && !file.startsWith("lib/"))
      .filter((file) => /from\s+["']\$lib\/[^"']*\.js["']/.test(readFileSync(resolve(process.cwd(), "src", file), "utf8")));
    expect(offenders).toEqual([]);
  });

  it("does not flatten radius, shadow or Tailwind spacing with a global reset", () => {
    expect(appCss).not.toMatch(/border-radius:\s*0px\s*!important/);
    expect(appCss).not.toMatch(/box-shadow:\s*none\s*!important/);
    expect(appCss).not.toMatch(/^\*\s*\{[^}]*padding:\s*0/m);
  });

  it("follows the OS appearance", () => {
    expect(appCss).toContain("@custom-variant dark (@media (prefers-color-scheme: dark));");
    expect(appCss).toContain("@media (prefers-color-scheme: dark) {");
  });

  it("no longer defines or reads the old telemetry theme variables or HUD classes", () => {
    const legacy = /--(bg-primary|bg-surface|bg-panel|border-raw|border-focus|text-primary|text-muted|font-mono|accent-industrial|error-color|success-color)\b|\bhud-btn\b/;
    const offenders = readdirSync(resolve(process.cwd(), "src"), { recursive: true })
      .filter((file) => /\.(svelte|css)$/.test(file) && !file.startsWith("lib/"))
      .filter((file) => legacy.test(readFileSync(resolve(process.cwd(), "src", file), "utf8")));
    expect(offenders).toEqual([]);
  });
});
