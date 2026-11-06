<script lang="ts">
  import type { InstanceInfo } from '../../../ui.shell/frontend/lib/api';
  import { appearance } from '../../../ui.shell/frontend/lib/theme';
  import { instanceIconKind, instanceIconSources } from '../lib/instance-icons';
  import vanilla from '../assets/instance-icons/vanilla.png';
  import forge from '../assets/instance-icons/forge.png';
  import neoforge from '../assets/instance-icons/neoforge.png';
  import fabric from '../assets/instance-icons/fabric.png';

  let { instance, custom }: { instance: InstanceInfo; custom?: string } = $props();
  const builtins = { vanilla, forge, neoforge, fabric };
  const kind = $derived(instanceIconKind(instance));
  const sources = $derived(instanceIconSources(custom, $appearance.active?.instanceIcons?.[kind], builtins[kind]));
  let failed = $state<string[]>([]);
  $effect(() => { sources; failed = []; });
  const visible = $derived(sources.find(source => !failed.includes(source)));
</script>

<span data-h-part="instance-icon" class="instance-icon" aria-hidden="true">
  {#each sources as source (source)}
    {#if source === visible}<img src={source} class:pixel={source === builtins[kind]} alt="" draggable="false" onerror={() => failed = [...failed, source]} />{/if}
  {/each}
</span>

<style>
  .instance-icon { display: inline-flex; align-items: center; justify-content: center; width: 36px; height: 36px; flex: 0 0 36px; }
  img { width: 100%; height: 100%; object-fit: contain; }
  .pixel { image-rendering: pixelated; }
</style>
