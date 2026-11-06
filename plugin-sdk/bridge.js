/** Isolated panel bridge. Import from a static module in ui/. */
const pending = new Map(), listeners = new Set(), themeListeners = new Set(), timers = new Set();
let themeLinks = [], themeRequest = 0, themeLoading;
async function applyAppearance(appearance) {
  const request = ++themeRequest;
  themeLoading?.cancel();
  const links = [];
  let timer, cancel;
  const stopped = new Promise((_, reject) => { cancel = () => reject(new Error('THEME_STALE')); timer = setTimeout(() => reject(new Error('THEME_SYNC_TIMEOUT')), 4500); });
  const loading = { links, cancel }; themeLoading = loading;
  try {
    const resources = Promise.all(appearance.styles.map(url => new Promise((resolve,reject) => {
      const link = document.createElement("link"); link.rel = "stylesheet"; link.href = url; link.media = "all"; links.push(link);
      link.onload = resolve; link.onerror = reject; document.head.append(link);
    }))).then(() => Promise.all(Array.from(document.fonts).map(font => font.load())));
    await Promise.race([resources, stopped]);
    if (closed || request !== themeRequest) { links.forEach(link => link.remove()); return; }
    document.documentElement.dataset.hPanelRoot = ""; document.documentElement.dataset.hSession = appearance.token;
    themeLinks.forEach(link => link.remove()); themeLinks = links; bridge.appearance = appearance; bridge.theme = appearance.theme;
    for (const listener of themeListeners) { try { listener(appearance); } catch (error) { console.error('Panel onTheme listener failed', error); } }
    port.postMessage({ event: "theme-ready", revision: appearance.revision, ok: true });
  } catch { links.forEach(link => link.remove()); if (!closed && request === themeRequest) port.postMessage({event:"theme-ready",revision:appearance.revision,ok:false}); }
  finally { clearTimeout(timer); for (const link of links) { link.onload = null; link.onerror = null; } if (themeLoading === loading) themeLoading = undefined; }
}
let port, nonce, next = 0, closed = false;
let resolveReady;
const ready = new Promise(resolve => { resolveReady = resolve; });
function close() {
  if (closed) return;
  closed = true; ++themeRequest; themeLoading?.cancel(); themeLoading?.links.forEach(link => link.remove()); themeLoading = undefined;
  themeLinks.forEach(link => link.remove()); themeLinks = []; themeListeners.clear(); listeners.clear();
  for (const pair of pending.values()) { clearTimeout(pair.timer); pair.reject(Object.assign(new Error('Panel closed'), { code: 'STALE_PANEL' })); }
  pending.clear();
  for (const timer of timers) clearTimeout(timer);
  timers.clear(); port?.close(); window.removeEventListener('message', handshake);
}
function handshake(event) {
  if (event.source !== parent || closed) return;
  if (event.data?.type === 'hydrogen:hello' && !nonce) {
    nonce = event.data.nonce; parent.postMessage({ type: 'hydrogen:ready', nonce }, '*');
  } else if (event.data?.type === 'hydrogen:port' && event.data.nonce === nonce && !port && event.ports.length === 1) {
    port = event.ports[0]; bridge.context = event.data.context ?? {}; bridge.locale = event.data.locale; bridge.theme = event.data.theme;
    port.onmessage = ({ data }) => {
      if (data.event === 'appearance') { void applyAppearance(data.appearance); return; }
      if (data.event === 'closed') { close(); return; }
      if (data.event === 'context') {
        bridge.context = data.context; bridge.locale = data.locale; bridge.theme = data.theme;
        for (const listener of listeners) listener(bridge.context); return;
      }
      const pair = pending.get(data.id); if (!pair) return;
      pending.delete(data.id); clearTimeout(pair.timer);
      if (data.ok) pair.resolve(data.result); else pair.reject(Object.assign(new Error(data.error?.message ?? 'Plugin call failed'), data.error));
    };
    resolveReady(bridge.context);
  }
}
async function call(command, params = null) {
  await ready;
  if (closed) throw Object.assign(new Error('Panel closed'), { code: 'STALE_PANEL' });
  if (pending.size >= 8) throw new Error('REQUEST_LIMIT');
  return new Promise((resolve, reject) => {
    const id = ++next;
    const timer = setTimeout(() => { pending.delete(id); reject(Object.assign(new Error('Request timed out'), { code: 'REQUEST_TIMEOUT' })); }, 35000);
    pending.set(id, { resolve, reject, timer });
    port.postMessage({ id, command, params });
  });
}
export const bridge = {
  ready, context: {}, locale: 'zh-CN', theme: 'light', appearance: null, call,
  onTheme(listener) { themeListeners.add(listener); return { dispose() { themeListeners.delete(listener); } }; },
  async resize(height) { await ready; if (!closed && Number.isFinite(height)) port.postMessage({ event: 'resize', height }); },
  onContext(listener) { listeners.add(listener); return { dispose() { listeners.delete(listener); } }; },
  tasks: {
    get: id => call('$tasks.get', { id }),
    cancel: id => call('$tasks.cancel', { id }),
    watch(id, listener, onerror = () => {}) {
      let disposed = false, timer;
      async function poll() {
        try {
          const task = await call('$tasks.get', { id });
          if (disposed || closed) return;
          listener(task);
          if (!['completed', 'failed', 'cancelled', 'exited'].includes(task.phase)) {
            timer = setTimeout(() => { timers.delete(timer); void poll(); }, 250); timers.add(timer);
          }
        } catch (error) { if (!disposed && !closed) onerror(error); }
      }
      void poll();
      return { dispose() { disposed = true; clearTimeout(timer); timers.delete(timer); } };
    }
  }
};
window.addEventListener('message', handshake);
window.addEventListener('pagehide', close, { once: true });
