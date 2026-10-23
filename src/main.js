import './app.css'
import App from './App.svelte'
import { mount } from 'svelte'

const app = mount(App, {
  target: document.getElementById('app'),
})

// Memory investigation only: `VITE_ZOOM_STRESS=1 npm run tauri build` (see devtools/zoomStress.js).
if (import.meta.env.VITE_ZOOM_STRESS === '1') {
  Promise.all([import('./devtools/zoomStress.js'), import('./services/platform/window.js')])
    .then(([stress, platform]) => stress.runZoomStress({ closeWindow: () => platform.getCurrentWindow().close() }))
    .catch((err) => console.error("Zoom stress failed:", err));
}

export default app
