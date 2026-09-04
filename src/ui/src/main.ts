import { mount } from 'svelte';
import './styles/tokens.css';
import './styles/base.css';
import App from './App.svelte';

// Horizon density is always compact — the density choice was removed globally.
// Drop any legacy stored preference so the layout renders compact with zero preference.
try {
  localStorage.removeItem('horizon:density');
} catch {
  // storage unavailable — the compact default below still applies
}
document.documentElement.setAttribute('data-density', 'compact');

const target = document.getElementById('app');
if (!target) {
  throw new Error('EspectralClient: <div id="app"> no encontrado en index.html');
}

const app = mount(App, { target });

export default app;
