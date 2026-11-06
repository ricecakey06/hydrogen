<script lang="ts">
  import { tick, type Snippet } from 'svelte';
  import type { Snapshot } from '../lib/api';
  import { contributions, uiState, type ViewContext } from '../lib/contributions';
  import PluginSlot from './PluginSlot.svelte';
  let { snapshot, location, context, children, onchange, onopen }: {
    snapshot: Snapshot | null; location: string; context: ViewContext; children: Snippet;
    onchange?: () => Promise<void>; onopen?: () => void;
  } = $props();
  let menu: HTMLDivElement;
  let x = $state(0), y = $state(0);
  async function open(event: MouseEvent | KeyboardEvent) {
    if (event instanceof KeyboardEvent && !(event.key === 'ContextMenu' || event.shiftKey && event.key === 'F10')) return;
    onopen?.();
    event.preventDefault(); event.stopPropagation();
    const bounds = (event.target as HTMLElement).getBoundingClientRect();
    await tick();
    if (!contributions(snapshot, location, context, $uiState).length) return;
    x = Math.max(8, Math.min(event instanceof MouseEvent ? event.clientX : bounds.left, innerWidth - 280));
    y = Math.max(8, Math.min(event instanceof MouseEvent ? event.clientY : bounds.bottom, innerHeight - 240));
    menu.showPopover();
    menu.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
  }
  function keys(event: KeyboardEvent) {
    const buttons = [...menu.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === 'ArrowDown' ? (index + 1) % buttons.length : event.key === 'ArrowUp' ? (index - 1 + buttons.length) % buttons.length : event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : -1;
    if (next >= 0) { event.preventDefault(); buttons[next]?.focus(); }
  }
</script>
<div class="context-target" role="presentation" oncontextmenu={open} onkeydown={open}>{@render children()}</div>
<div bind:this={menu} popover="auto" role="presentation" onkeydown={keys} style:left={`${x}px`} style:top={`${y}px`}>
  <PluginSlot {snapshot} {location} {context} {onchange} menu ondone={() => menu.hidePopover()} />
</div>
<style>
  .context-target { display: contents; }
  [popover] { margin: 0; width: 260px; max-height: 224px; overflow: auto; padding: 8px; border: 1px solid var(--stroke); border-radius: 10px; background: var(--surface, white); color: inherit; box-shadow: 0 8px 30px color-mix(in srgb,var(--h-text-primary) 12%,transparent); }
</style>
