import { describe, expect, it } from "vitest";
import { buildAnnotationMarkdown } from "./annotationMarkdown.js";

describe("buildAnnotationMarkdown", () => {
  it("includes each supported annotation in a stable page section", () => {
    const { content, totalNotes } = buildAnnotationMarkdown({
      fileName: "source.pdf",
      createdAt: new Date("2026-08-02T12:00:00Z"),
      annotationsMap: {
        0: [
          { tool_type: "text", text: "Hatırla", color: "#fff" },
          { tool_type: "pen", points: [{}, {}] },
          { tool_type: "arrow", x: 1, y: 2, end_x: 3, end_y: 4 },
        ],
      },
    });

    expect(totalNotes).toBe(3);
    expect(content).toContain("# Belge Notları & Vurguları: source.pdf");
    expect(content).toContain("## Sayfa 1");
    expect(content).toContain('**Metin Notu**: "Hatırla"');
    expect(content).toContain("**Serbest Çizim**: 2 nokta");
    expect(content).toContain("**Ok Çizimi**: (1, 2) -> (3, 4)");
  });

  it("reports zero notes for an empty annotation map", () => {
    expect(buildAnnotationMarkdown({ fileName: "empty.pdf", annotationsMap: {} }).totalNotes).toBe(0);
  });
});
