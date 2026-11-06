<script lang="ts">
  import { t } from '../lib/i18n';
  import { onMount } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { invoke, errorMessage, type RunReport, type PanelSession } from '../lib/api';
  import Icon from './Icon.svelte';
  import { get } from 'svelte/store';
  import { appearance, cancelTheme, themeError, beginPanelTheme, acknowledgePanelTheme, closeThemePanel, type ThemeState } from '../lib/theme';
  let { pluginId, panelId, context = {}, locale = 'zh-CN', height = 350, onerror }: { pluginId: string; panelId: string; context?: Record<string, unknown>; locale?: string; height?: number; onerror: (message: string) => void } = $props();
  let session = $state<PanelSession | null>(null);
  let frame = $state<HTMLIFrameElement>();
  let container: HTMLDivElement;
  let port: MessagePort | undefined;
  let disposed = false;
  let handshaken = false;
  let loads = 0;
  let pending = 0;
  let themeTimer: ReturnType<typeof setTimeout> | undefined;
  let themeRevision = -1;
  let themeToken = '';
  function themeFailed(revision: number) {
    if (disposed || revision !== themeRevision || !session) return;
    acknowledgePanelTheme(session.sessionId,revision,false);
    stopped = true; revoke();
    themeError.set('THEME_SYNC_TIMEOUT: 面板未确认主题，已关闭失同步面板并请求撤销。');
    void cancelTheme().catch(error => onerror(errorMessage(error)));
  }
  function themeMessage(state: ThemeState) {
    if (!port || disposed || !session || session.appearance === "independent" || !state.active) return;
    if (state.revision === themeRevision) return;
    const themeUrl = new URL(session.url); themeUrl.pathname = `/${session.sessionId}/@theme/${state.active.token}/`;
    const prefix = themeUrl.toString();
    const bound = session, visual = state.active, revision = state.revision;
    clearTimeout(themeTimer); themeRevision = revision; themeToken = visual.token;
    beginPanelTheme(session.sessionId,state.revision);
    themeTimer = setTimeout(() => themeFailed(revision), 5000);
    void invoke('panel_theme', { sessionId: bound.sessionId, revision, token: visual.token, ready: false }).then(() => {
      if (disposed || revision !== themeRevision) return;
      port?.postMessage({ event: "appearance", appearance: { theme: visual.scheme, id: visual.actualId, mode: visual.choice.mode, revision, highContrast: visual.highContrast, token: visual.token, styles: [prefix + "tokens.css", prefix + "panels.css"] } });
    }).catch(() => themeFailed(revision));
  }
  let requestedHeight = $state<number>();
  let stopped = $state(false);
  function revoke() {
    if (disposed) return;
    disposed = true; clearTimeout(themeTimer); if (session) closeThemePanel(session.sessionId);
    port?.postMessage({ event: 'closed' });
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
      if (disposed) return;
      if (message.data?.event === 'theme-ready') {
        const revision = message.data.revision, token = themeToken;
        if (revision !== themeRevision) return;
        if (!message.data.ok) { themeFailed(revision); return; }
        try {
          await invoke('panel_theme', { sessionId: bound.sessionId, revision, token, ready: true });
          if (!disposed && revision === themeRevision) { clearTimeout(themeTimer); acknowledgePanelTheme(bound.sessionId,revision,true); }
        } catch { themeFailed(revision); }
        return;
      }
      if (message.data?.event === 'resize') {
        if (Number.isFinite(message.data.height)) requestedHeight = Math.max(120, Math.min(900, message.data.height));
        return;
      }
      if (pending >= 8) { port?.postMessage({ id: message.data?.id, ok: false, error: { code: 'REQUEST_LIMIT', message: '面板请求过多。' } }); return; }
      const data = message.data;
      if (!data || !Number.isSafeInteger(data.id) || typeof data.command !== 'string' || data.command.length > 100) return;
      pending++;
      try {
        if (JSON.stringify(data.params ?? null).length > 262144) throw { code: 'MESSAGE_LIMIT', message: '请求超过大小限制。' };
        const result = await invoke<RunReport>('panel_call', { sessionId: bound.sessionId, generation: bound.generation, command: data.command, params: data.params ?? null });
        if (!disposed) port?.postMessage({ id: data.id, ok: true, result: result.result });
      } catch (error) {
        if (!disposed) port?.postMessage({ id: data.id, ok: false, error: error && typeof error === 'object' ? error : { code: 'PANEL_ERROR', message: errorMessage(error) } });
      } finally { pending--; }
    };
    frame?.contentWindow?.postMessage({ type: 'hydrogen:port', nonce: session.nonce, context, locale, theme: get(appearance).active?.scheme ?? 'light' }, '*', [channel.port2]);
    themeMessage(get(appearance));
  }
  $effect(() => { port?.postMessage({ event: 'context', context, locale, theme: get(appearance).active?.scheme ?? 'light' }); });
  onMount(() => {
    const unsubscribeTheme = appearance.subscribe(themeMessage);
    const unlisten = listen<{ pluginId: string; generation: number }>('plugin-ui-stop', ({ payload }) => {
      if (payload.pluginId === pluginId && payload.generation === session?.generation) {
        revoke(); stopped = true;
      }
    });
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
    return () => { unsubscribeTheme(); void unlisten.then(dispose => dispose()); document.removeEventListener('outrostart', leaving, true); window.removeEventListener('message', handshake); revoke(); };
  });
</script>

<div bind:this={container}>
{#if stopped}
  <p role="status">{t("插件不可用，请重新打开。")}</p>
{:else if session}
  <iframe bind:this={frame} src={session.url} sandbox="allow-scripts" style:height={(requestedHeight ?? height) + "px"} title={t("插件自定义面板")} onload={loaded}></iframe>
{:else}
  <p class="operation-status" role="status"><span class="spinner"><Icon name="spinner" size={18} /></span>{t("正在打开隔离面板…")}</p>
{/if}
</div>

<style>iframe { width: 100%; height: 350px; border: 1px solid var(--stroke); border-radius: 8px; background: var(--h-surface-card); }</style>
