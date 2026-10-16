import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { render, screen } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import { Button } from "$lib/components/ui/button/index.js";
import { Toggle } from "$lib/components/ui/toggle/index.js";

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

  it("does not flatten radius, shadow or Tailwind spacing with a global reset", () => {
    expect(appCss).not.toMatch(/border-radius:\s*0px\s*!important/);
    expect(appCss).not.toMatch(/box-shadow:\s*none\s*!important/);
    expect(appCss).not.toMatch(/^\*\s*\{[^}]*padding:\s*0/m);
  });

  it("follows the OS appearance and points legacy variables at the palette", () => {
    expect(appCss).toContain("@custom-variant dark (@media (prefers-color-scheme: dark));");
    expect(appCss).toContain("@media (prefers-color-scheme: dark) {");
    expect(appCss).not.toContain("--accent-industrial: #");
    expect(appCss).toContain("--accent-industrial: var(--primary);");
    expect(appCss).toContain("--bg-surface: var(--card);");
  });
});
