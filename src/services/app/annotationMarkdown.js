export function buildAnnotationMarkdown({ fileName, annotationsMap, createdAt = new Date() }) {
  let content = `# Belge Notları & Vurguları: ${fileName}\n\n`;
  content += `*Oluşturulma Tarihi: ${createdAt.toLocaleString()}*\n\n---\n\n`;
  let totalNotes = 0;

  Object.keys(annotationsMap).forEach((pageIndex) => {
    const annotations = annotationsMap[pageIndex];
    if (!annotations?.length) return;

    content += `## Sayfa ${Number.parseInt(pageIndex, 10) + 1}\n`;
    annotations.forEach((annotation) => {
      totalNotes += 1;
      if (annotation.tool_type === "text") {
        content += `- 📝 **Metin Notu**: "${annotation.text}" *(Renk: ${annotation.color})*\n`;
      } else if (annotation.tool_type === "highlighter") {
        content += `- 🟡 **Fosforlu Vurgu**: Vurgulanan alan *(Renk: ${annotation.color})*\n`;
      } else if (annotation.tool_type === "pen") {
        content += `- ✒️ **Serbest Çizim**: ${annotation.points?.length || 0} nokta\n`;
      } else if (annotation.tool_type === "rectangle") {
        content += `- ⏹️ **Dikdörtgen Vurgu**: (${annotation.x}, ${annotation.y}) [${annotation.w}x${annotation.h}]\n`;
      } else if (annotation.tool_type === "arrow") {
        content += `- ↗️ **Ok Çizimi**: (${annotation.x}, ${annotation.y}) -> (${annotation.end_x}, ${annotation.end_y})\n`;
      }
    });
    content += "\n";
  });

  return { content, totalNotes };
}
