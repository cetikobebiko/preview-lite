import { afterEach, describe, expect, it } from "vitest";
import { findScrollRoot } from "./scrollRoot.js";

afterEach(() => {
  document.body.innerHTML = "";
});

describe("findScrollRoot", () => {
  it("returns the nearest vertically scrolling ancestor", () => {
    document.body.innerHTML = `
      <div id="outer" style="overflow-y: scroll">
        <div id="list" style="overflow-y: auto">
          <div><div id="item"></div></div>
        </div>
      </div>`;
    expect(findScrollRoot(document.getElementById("item"))).toBe(document.getElementById("list"));
  });

  it("skips clipping-only ancestors and falls back to the viewport", () => {
    document.body.innerHTML = `<div style="overflow: hidden"><div id="item"></div></div>`;
    expect(findScrollRoot(document.getElementById("item"))).toBeNull();
    expect(findScrollRoot(null)).toBeNull();
  });
});
