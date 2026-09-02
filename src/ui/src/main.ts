import { mount } from 'svelte';
import './styles/tokens.css';
import './styles/base.css';
import App from './App.svelte';

const target = document.getElementById('app');
if (!target) {
  throw new Error('EspectralClient: <div id="app"> no encontrado en index.html');
}

const app = mount(App, { target });

export default app;
