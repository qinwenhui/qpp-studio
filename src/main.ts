import { mount } from 'svelte';
import './app.css';
import { disableDefaultContextMenu } from '$lib/disableContextMenu';
import App from './App.svelte';

disableDefaultContextMenu();

const app = mount(App, {
  target: document.getElementById('app')!,
});

export default app;
