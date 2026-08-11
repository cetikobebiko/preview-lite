import './app.css'
import App from './App.svelte'
import { mount } from 'svelte'
import { open } from '@tauri-apps/plugin-shell';

document.addEventListener('click', (e) => {
  const target = e.target.closest('a');
  if (target && target.href && (target.href.startsWith('http://') || target.href.startsWith('https://'))) {
    e.preventDefault();
    open(target.href).catch(err => console.error("Failed to open link:", err));
  }
});

const app = mount(App, {
  target: document.getElementById('app'),
})

export default app
