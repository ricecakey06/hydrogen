<script lang="ts">
  import Icon from './Icon.svelte';
  import { skinHead } from '../lib/avatar';

  let { skin, size, iconSize = size, iconWidth = 0.8 }: {
    skin?: string | null; size: number; iconSize?: number; iconWidth?: number;
  } = $props();
  let face = $state('');
  const skinSource = $derived(skin ?? '');

  $effect(() => {
    const source = skinSource; face = '';
    if (!source) return;
    const image = new Image(); let active = true;
    image.onload = () => { if (!active) return; try { face = skinHead(image); } catch { face = ''; } };
    image.onerror = () => { if (active) face = ''; };
    image.src = source;
    return () => { active = false; image.onload = null; image.onerror = null; };
  });
</script>

{#if face}<img src={face} alt="玩家头像" width={size} height={size} />{:else}<Icon name="user" size={iconSize} width={iconWidth} />{/if}
