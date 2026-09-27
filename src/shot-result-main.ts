import { mount } from 'svelte';
import './app.css';
import { disableDefaultContextMenu } from '$lib/disableContextMenu';
import ShotResult from './lib/components/screenshot/ShotResult.svelte';

disableDefaultContextMenu();

const app = mount(ShotResult, {
  target: document.getElementById('app')!,
});

export default app;
