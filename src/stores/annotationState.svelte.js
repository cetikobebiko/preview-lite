export function createAnnotationState() {
  const state = $state({
    activeTool: "pointer", activeColor: "#facc15", strokeColor: "#ff3b30", strokeSize: 3,
    annotationsMap: {}, isDrawing: false, currentPoints: [], startPos: { x: 0, y: 0 }, currentAnnoItem: null,
  });
  const fields = ["activeTool", "activeColor", "strokeColor", "strokeSize", "annotationsMap", "isDrawing", "currentPoints", "startPos", "currentAnnoItem"];
  const api = {};
  for (const field of fields) Object.defineProperty(api, field, { enumerable: true, get: () => state[field], set: (value) => { state[field] = value; } });
  return api;
}
