import '@xyflow/svelte/dist/style.css';
import './app.css';
import { mount } from 'svelte';
import App from './App.svelte';

const target = document.getElementById('app');
if (!target) {
  throw new Error('knect WebUI root element #app is missing');
}

const app = mount(App, { target });

export default app;
