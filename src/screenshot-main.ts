import { mount } from 'svelte';
import './app.css';
import { disableDefaultContextMenu } from '$lib/disableContextMenu';
import SelectionOverlay from './lib/components/screenshot/SelectionOverlay.svelte';

disableDefaultContextMenu();

const app = mount(SelectionOverlay, {
  target: document.getElementById('app')!,
});

export default app;
