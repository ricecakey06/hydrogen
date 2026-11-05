<script lang="ts">
  import { t } from '../lib/i18n';
  import { onMount } from 'svelte';
  import { invoke, errorMessage, type PanelSession } from '../lib/api';
  import Icon from './Icon.svelte';
  let { pluginId, panelId, onerror }: { pluginId: string; panelId: string; onerror: (message: string) => void } = $props();
  let session = $state<PanelSession | null>(null);
  let frame = $state<HTMLIFrameElement>();
  let container: HTMLDivElement;
  let port: MessagePort | undefined;
  let disposed = false;
  let handshaken = false;
  let loads = 0;
  let pending = 0;
  function revoke() {
    if (disposed) return;
    disposed = true;
    port?.close();
    if (session) void invoke('panel_close', { sessionId: session.sessionId }).catch(() => {});
  }
  function loaded() {
    if (!session || disposed) return;
    if (++loads > 1) { revoke(); onerror(t("面板发生导航，会话已撤销。请重新打开。")); return; }
    frame?.contentWindow?.postMessage({ type: 'hydrogen:hello', nonce: session.nonce }, '*');
  }
  function handshake(event: MessageEvent) {
    if (disposed || handshaken || !session || event.source !== frame?.contentWindow || event.origin !== 'null'
        || event.data?.type !== 'hydrogen:ready' || event.data.nonce !== session.nonce) return;
    handshaken = true;
    const channel = new MessageChannel();
    port = channel.port1;
    const bound = session;
    port.onmessage = async (message: MessageEvent) => {
      if (disposed || pending >= 8) return;
      const data = message.data;
      if (!data || !Number.isSafeInteger(data.id) || typeof data.command !== 'string' || data.command.length > 100) return;
      pending++;
      try {
        const result = await invoke('panel_call', { sessionId: bound.sessionId, generation: bound.generation, command: data.command });
        if (!disposed) port?.postMessage({ id: data.id, ok: true, result });
      } catch (error) {
        if (!disposed) port?.postMessage({ id: data.id, ok: false, error: errorMessage(error) });
      } finally { pending--; }
    };
    frame?.contentWindow?.postMessage({ type: 'hydrogen:port', nonce: session.nonce }, '*', [channel.port2]);
  }
  onMount(() => {
    // Revoke at the start of a page exit, without waiting for its visual outro to finish.
    function leaving(event: Event) {
      if (event.target instanceof Element && event.target.contains(container)) revoke();
    }
    document.addEventListener('outrostart', leaving, true);
    window.addEventListener('message', handshake);
    void invoke<PanelSession>('panel_open', { pluginId, panel: panelId }).then(value => {
      if (disposed) void invoke('panel_close', { sessionId: value.sessionId });
      else session = value;
    }).catch(error => onerror(errorMessage(error)));
    return () => { document.removeEventListener('outrostart', leaving, true); window.removeEventListener('message', handshake); revoke(); };
  });
</script>

<div bind:this={container}>
{#if session}
  <iframe bind:this={frame} src={session.url} sandbox="allow-scripts" title={t("插件自定义面板")} onload={loaded}></iframe>
{:else}
  <p class="operation-status" role="status"><span class="spinner"><Icon name="spinner" size={18} /></span>{t("正在打开隔离面板…")}</p>
{/if}
</div>

<style>iframe { width: 100%; height: 350px; border: 1px solid var(--stroke); border-radius: 8px; background: white; }</style>
