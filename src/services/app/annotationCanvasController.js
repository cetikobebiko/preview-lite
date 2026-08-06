/** Owns annotation-pointer state transitions while the annotation store remains authoritative. */
export function createAnnotationCanvasController({ annotation, viewer, getCanvases, promptText, render }) {
  function getCanvasCoords(event) {
    const { left, right } = getCanvases();
    const canvas = viewer.isBookMode && event.target === right ? right : left;
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    if (!rect.width || !rect.height || !canvas.width || !canvas.height) return null;
    return {
      x: (event.clientX - rect.left) * (canvas.width / rect.width),
      y: (event.clientY - rect.top) * (canvas.height / rect.height),
    };
  }

  function addToCurrentPage(item) {
    if (!annotation.annotationsMap[viewer.currentPageIndex]) annotation.annotationsMap[viewer.currentPageIndex] = [];
    annotation.annotationsMap[viewer.currentPageIndex].push(item);
    annotation.annotationsMap = { ...annotation.annotationsMap };
    render({ silent: true });
  }

  function getCanvasSize() {
    const { left } = getCanvases();
    return { width: left?.width ?? 0, height: left?.height ?? 0 };
  }

  async function handleMouseDown(event) {
    if (annotation.activeTool === "pointer") return;
    const coords = getCanvasCoords(event);
    if (!coords) return;
    annotation.isDrawing = true;
    annotation.startPos = coords;
    const canvas = getCanvasSize();

    if (annotation.activeTool === "pen" || annotation.activeTool === "highlighter") {
      annotation.currentPoints = [coords];
      annotation.currentAnnoItem = {
        page_index: viewer.currentPageIndex, tool_type: annotation.activeTool, points: annotation.currentPoints,
        color: annotation.strokeColor, size: annotation.strokeSize, canvas_width: canvas.width, canvas_height: canvas.height,
      };
    } else if (annotation.activeTool === "text") {
      const userText = await promptText({ title: "Metin Açıklaması", label: "Metin" });
      if (userText) addToCurrentPage({
        page_index: viewer.currentPageIndex, tool_type: "text", x: coords.x, y: coords.y, text: userText,
        color: annotation.strokeColor, size: annotation.strokeSize, canvas_width: canvas.width, canvas_height: canvas.height,
      });
      annotation.isDrawing = false;
    }
  }

  function handleMouseMove(event) {
    if (!annotation.isDrawing || annotation.activeTool === "pointer") return;
    const coords = getCanvasCoords(event);
    if (!coords) return;
    const canvas = getCanvasSize();
    if (annotation.activeTool === "pen" || annotation.activeTool === "highlighter") {
      annotation.currentPoints.push(coords);
      annotation.currentAnnoItem.points = annotation.currentPoints;
    } else if (annotation.activeTool === "rectangle") {
      annotation.currentAnnoItem = {
        page_index: viewer.currentPageIndex, tool_type: "rectangle",
        x: Math.min(annotation.startPos.x, coords.x), y: Math.min(annotation.startPos.y, coords.y),
        w: Math.abs(coords.x - annotation.startPos.x), h: Math.abs(coords.y - annotation.startPos.y),
        color: annotation.strokeColor, size: annotation.strokeSize, canvas_width: canvas.width, canvas_height: canvas.height,
      };
    } else if (annotation.activeTool === "arrow") {
      annotation.currentAnnoItem = {
        page_index: viewer.currentPageIndex, tool_type: "arrow", x: annotation.startPos.x, y: annotation.startPos.y,
        end_x: coords.x, end_y: coords.y, color: annotation.strokeColor, size: annotation.strokeSize,
        canvas_width: canvas.width, canvas_height: canvas.height,
      };
    }
    render({ silent: true });
  }

  function handleMouseUp() {
    if (!annotation.isDrawing) return;
    annotation.isDrawing = false;
    if (annotation.currentAnnoItem) {
      addToCurrentPage(annotation.currentAnnoItem);
      annotation.currentAnnoItem = null;
      annotation.currentPoints = [];
    }
  }

  return Object.freeze({ handleMouseDown, handleMouseMove, handleMouseUp, addToCurrentPage });
}
