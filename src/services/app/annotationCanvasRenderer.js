/** Paints authoritative annotation-store items onto an already-rendered image canvas. */
export function paintAnnotations({ canvas, pageIndex, annotation, currentPageIndex }) {
  if (!canvas) return;
  const context = canvas.getContext("2d", { alpha: false });
  if (!context) return;
  for (const item of annotation.annotationsMap[pageIndex] || []) paintAnnotation(context, item);
  if (annotation.isDrawing && annotation.currentAnnoItem && pageIndex === currentPageIndex) {
    paintAnnotation(context, annotation.currentAnnoItem);
  }
}

function paintAnnotation(context, item) {
  context.save();
  context.strokeStyle = item.color;
  context.fillStyle = item.color;
  context.lineWidth = item.tool_type === "highlighter" ? item.size * 3 : item.size;
  context.lineCap = "round";
  context.lineJoin = "round";
  context.globalAlpha = item.tool_type === "highlighter" ? 0.4 : 1;

  switch (item.tool_type) {
    case "pen":
    case "highlighter":
      if (item.points?.length > 1) {
        context.beginPath();
        context.moveTo(item.points[0].x, item.points[0].y);
        for (let index = 1; index < item.points.length; index += 1) context.lineTo(item.points[index].x, item.points[index].y);
        context.stroke();
      }
      break;
    case "rectangle":
      if (item.x !== undefined && item.y !== undefined && item.w !== undefined && item.h !== undefined) {
        context.beginPath(); context.rect(item.x, item.y, item.w, item.h); context.stroke();
      }
      break;
    case "arrow":
      if (item.x !== undefined && item.y !== undefined && item.end_x !== undefined && item.end_y !== undefined) {
        paintArrow(context, item.x, item.y, item.end_x, item.end_y, item.size);
      }
      break;
    case "text":
      if (item.x !== undefined && item.y !== undefined && item.text) {
        context.font = `${Math.max(14, item.size * 3)}px Inter, sans-serif`;
        context.fillText(item.text, item.x, item.y);
      }
      break;
  }
  context.restore();
}

function paintArrow(context, fromX, fromY, toX, toY, width) {
  const headLength = Math.max(12, width * 3);
  const angle = Math.atan2(toY - fromY, toX - fromX);
  context.beginPath(); context.moveTo(fromX, fromY); context.lineTo(toX, toY); context.stroke();
  context.beginPath(); context.moveTo(toX, toY);
  context.lineTo(toX - headLength * Math.cos(angle - Math.PI / 6), toY - headLength * Math.sin(angle - Math.PI / 6));
  context.lineTo(toX - headLength * Math.cos(angle + Math.PI / 6), toY - headLength * Math.sin(angle + Math.PI / 6));
  context.closePath(); context.fill();
}
