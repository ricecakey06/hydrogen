import { get, writable } from 'svelte/store';
import { action, type Settings, errorMessage } from './api.ts';

export type ThemeChoice = Settings['theme'];
export interface ThemeEntry { id: string; title: string; pluginId: string | null; variants: string[]; available: boolean; preview?: string | null }
export interface ThemeVisual {
  token: string; choice: ThemeChoice; actualId: string; scheme: 'light' | 'dark'; highContrast: boolean;
  packageDigest: string | null; fallbackReason: string | null; styles: string[]; logo: string | null;
  instanceIcons?: Partial<Record<'vanilla' | 'forge' | 'neoforge' | 'fabric', string>>;
  layers: { id: string; slot: string; source: string }[]; images: string[]; fonts: string[]; fontSources: string[];
  bytes: number; decodedBytes: number;
}
export interface ThemeState { revision: number; active: ThemeVisual | null; preview: boolean; remainingSeconds: number; resources: number; snapshots: number; status: string }
export const appearance = writable<ThemeState>({ revision: 0, active: null, preview: false, remainingSeconds: 0, resources: 0, snapshots: 0, status: 'starting' });
export const themeError = writable('');
const dark = () => matchMedia('(prefers-color-scheme: dark)').matches;
const contrast = () => matchMedia('(forced-colors: active)').matches;
const sessions = new Map<string, VisualSession>();
const panelSync = new Map<string, { revision: number; promise: Promise<boolean>; resolve: (ok: boolean) => void }>();
export function beginPanelTheme(id: string, revision: number) {
  panelSync.get(id)?.resolve(false);
  let resolve!: (ok: boolean) => void;
  const promise = new Promise<boolean>(done => { resolve = done; });
  panelSync.set(id, { revision, promise, resolve });
}
export function acknowledgePanelTheme(id: string, revision: number, ok: boolean) {
  const pending = panelSync.get(id);
  if (pending?.revision === revision) pending.resolve(ok);
}
export function closeThemePanel(id: string) { panelSync.get(id)?.resolve(true); panelSync.delete(id); }
let root: HTMLElement | undefined;
let requested = 0;
let queue: Promise<void> = Promise.resolve();
let disposed = false;
let preparing: VisualSession | undefined;
let settingsKey = '';
let confirmedChoice: ThemeChoice = { id: 'builtin:default', mode: 'system', material: 'auto' };
let renderingRevision = 0;

export function timeout<T>(promise: Promise<T>, milliseconds: number, signal?: AbortSignal): Promise<T> {
  return new Promise((resolve, reject) => {
    const stop = () => { clearTimeout(timer); signal?.removeEventListener('abort', cancel); };
    const cancel = () => { stop(); reject(new Error('THEME_STALE')); };
    const timer = setTimeout(() => { stop(); reject(new Error('THEME_SYNC_TIMEOUT')); }, milliseconds);
    if (signal?.aborted) { cancel(); return; }
    signal?.addEventListener('abort', cancel, { once: true });
    promise.then(value => { stop(); resolve(value); }, error => { stop(); reject(error); });
  });
}

export class VisualSession {
  readonly controller = new AbortController();
  readonly links: HTMLLinkElement[] = [];
  readonly nodes: HTMLElement[] = [];
  readonly loadingImages: HTMLImageElement[] = [];
  private preparation?: Promise<void>;
  closed = false;
  readonly visual: ThemeVisual;
  constructor(visual: ThemeVisual) { this.visual = visual; }
  prepare() { return this.preparation ??= this.prepareResources(); }
  private async prepareResources() {
    if (this.closed) throw new Error('THEME_STALE');
    const signal = this.controller.signal;
    const styles = this.visual.styles.map(url => new Promise<void>((resolve, reject) => {
      const link = document.createElement('link'); link.rel = 'stylesheet'; link.href = url; link.media = 'all';
      link.dataset.hOwner = this.visual.token; this.links.push(link);
      link.onload = () => resolve(); link.onerror = () => reject(new Error('THEME_RESOURCE: stylesheet'));
      document.head.append(link);
    }));
    const images = this.visual.images.map(url => { const image = new Image(); this.loadingImages.push(image); image.src = url; return image.decode(); });
    const fonts = (this.visual.fontSources ?? []).map((url, index) => new FontFace(`h-probe-${this.visual.token}-${index}`, `url("${url}")`).load());
    await timeout(Promise.all([...styles, ...images, ...fonts]), 5000, signal);
    await timeout(Promise.all(this.visual.fonts.map(name => document.fonts.load(`16px "${name}"`))), 1500, signal);
    if (this.closed) throw new Error('THEME_STALE');
    for (const layer of this.visual.layers) {
      const image = new Image(); image.src = layer.source; image.alt = ''; image.draggable = false;
      image.setAttribute('aria-hidden', 'true'); image.dataset.hLayer = layer.id; image.dataset.hOwner = this.visual.token;
      image.dataset.hSlot = layer.slot; this.nodes.push(image);
    }
  }
  mount() {
    if (this.closed || !root) throw new Error('THEME_STALE');
    const destinations = this.nodes.map(node => {
      const slot = root!.querySelector<HTMLElement>(`[data-h-theme-slot="${node.dataset.hSlot}"]`);
      if (!slot) throw new Error(`THEME_PART: ${node.dataset.hSlot}`);
      return slot;
    });
    for (const link of this.links) link.media = 'all';
    root.dataset.hSession = this.visual.token;
    root.dataset.hScheme = this.visual.scheme;
    this.nodes.forEach((node, index) => destinations[index].append(node));
    const logo = root.querySelector<HTMLImageElement>('[data-h-part="brand-image"]');
    const builtin = root.querySelector<HTMLElement>('[data-h-builtin-brand]');
    if (logo && builtin) {
      logo.hidden = !this.visual.logo; builtin.hidden = !!this.visual.logo;
      if (this.visual.logo) { logo.src = this.visual.logo; logo.onerror = () => { if (this.closed || root?.dataset.hSession !== this.visual.token) return; logo.hidden = true; builtin.hidden = false; themeError.set('THEME_RESOURCE: 标志渲染失败，已显示内置标志。'); }; }
      else { logo.removeAttribute('src'); logo.onerror = null; }
    }
  }
  unmount() { for (const link of this.links) link.media = 'not all'; for (const node of this.nodes) node.remove(); }
  dispose() {
    if (this.closed) return;
    this.closed = true; this.controller.abort();
    const errors: unknown[] = [];
    for (const link of this.links) { link.onload = null; link.onerror = null; }
    for (const image of this.loadingImages) image.removeAttribute('src');
    for (const item of [...this.nodes, ...this.links].reverse()) { try { item.remove(); } catch (error) { errors.push(error); } }
    const incomplete = (message: string) => { appearance.update(state => ({ ...state, status: 'cleanup-incomplete' })); themeError.set(message); };
    if (errors.length) incomplete('THEME_CLEANUP_INCOMPLETE: 请使用安全主题入口恢复。');
    void action('themeRelease', { token: this.visual.token }).catch(() => incomplete('THEME_CLEANUP_INCOMPLETE: 资源租约未确认释放。'));
  }
}

function anchors() {
  return Array.from(root?.querySelectorAll<HTMLElement>('.scroll-pane') ?? []).map(pane => {
    const boundary = pane.getBoundingClientRect();
    const element = Array.from(pane.children).find(node => node.getBoundingClientRect().bottom > boundary.top);
    return { pane, element, top: element?.getBoundingClientRect().top ?? boundary.top, scroll: pane.scrollTop };
  });
}
async function render(state: ThemeState) {
  const visual = state.active;
  if (!root || !visual || disposed || state.revision < renderingRevision) return;
  renderingRevision = state.revision;
  let session = sessions.get(visual.token);
  if (!session) {
    session = new VisualSession(visual); sessions.set(visual.token, session);
    try { await session.prepare(); } catch (e) { sessions.delete(visual.token); session.dispose(); throw e; }
  }
  if (disposed || state.revision < renderingRevision) return;
  const old = get(appearance).active?.token;
  if (old !== visual.token) {
    const positions = anchors();
    if (old) sessions.get(old)?.unmount();
    session.mount();
    for (const { pane, element, top, scroll } of positions) {
      pane.scrollTop = element?.isConnected ? scroll + element.getBoundingClientRect().top - top : scroll;
    }
  }
  appearance.set({ ...state, status: 'syncing' });
  const waiting = [...panelSync.values()].filter(sync => sync.revision === state.revision);
  if (waiting.length) {
    const results = await timeout(Promise.all(waiting.map(sync => sync.promise)), 5000);
    if (disposed || state.revision < renderingRevision) return;
    if (results.some(ok => !ok)) throw new Error('THEME_SYNC_TIMEOUT');
  }
  if (disposed || state.revision < renderingRevision) return;
  appearance.set(state);
  if (!state.preview) {
    for (const [token, oldSession] of sessions) if (token !== visual.token) { oldSession.dispose(); sessions.delete(token); }
  }
  window.dispatchEvent(new CustomEvent('hydrogen-theme', { detail: state }));
}

export function chooseTheme(choice: ThemeChoice, preview = true, startup = false): Promise<void> {
  const request = ++requested;
  preparing?.controller.abort();
  const job = async () => {
    if (request !== requested || disposed) return;
    themeError.set('');
    let candidate: VisualSession | undefined;
    const started = performance.now();
    try {
      if (get(appearance).preview) await render(await action<ThemeState>('themeCancel'));
      const preparation = action<ThemeVisual>('themePrepare', { choice, systemDark: dark(), highContrast: contrast() });
      let visual: ThemeVisual;
      try { visual = await timeout(preparation, startup ? 1000 : 8000); }
      catch (error) { void preparation.then(late => action('themeRelease', { token: late.token })).catch(() => {}); throw error; }
      if (request !== requested || disposed) { await action('themeRelease', { token: visual.token }); return; }
      candidate = new VisualSession(visual); preparing = candidate; sessions.set(visual.token, candidate);
      await timeout(candidate.prepare(), startup ? Math.max(1,1000-(performance.now()-started)) : 5500, candidate.controller.signal);
      if (request !== requested || disposed) { candidate.dispose(); sessions.delete(visual.token); return; }
      const state = await action<ThemeState>('themeApply', { token: visual.token, preview });
      if (request !== requested || disposed) { const restored = await action<ThemeState>('themeCancel'); candidate.dispose(); sessions.delete(visual.token); if (!disposed) await render(restored); return; }
      await render(state);
      preparing = undefined;
    } catch (e) {
      preparing = undefined;
      if (request === requested && !disposed) {
        themeError.set(errorMessage(e));
        const restored = await action<ThemeState>('themeCancel').catch(() => null);
        if (restored?.active) await render(restored).catch(() => {});
      }
      if (candidate && get(appearance).active?.token !== candidate.visual.token) { candidate.dispose(); sessions.delete(candidate.visual.token); }
    }
  };
  queue = queue.then(job, job);
  return queue;
}
export async function confirmTheme() {
  if (get(appearance).status === 'syncing') return;
  const token = get(appearance).active?.token;
  if (!token) return;
  try { const state = await action<ThemeState>('themeConfirm', { token }); confirmedChoice = state.active!.choice; await render(state); }
  catch (e) { themeError.set(errorMessage(e)); await cancelTheme(); }
}
export async function cancelTheme() { ++requested; preparing?.controller.abort(); try { await render(await action<ThemeState>('themeCancel')); } catch (error) { themeError.set(errorMessage(error)); } }
export async function restoreTheme() {
  ++requested; preparing?.controller.abort();
  let saveError = '';
  try { await action('themeRestore'); } catch (error) { saveError = errorMessage(error); }
  confirmedChoice = { id: 'builtin:default', mode: 'system', material: 'auto' };
  await chooseTheme(confirmedChoice, false);
  if (saveError) themeError.set(`THEME_INVALID: 本次已恢复内置外观，恢复偏好未保存：${saveError}`);
}
export function syncThemeSettings(settings: Settings, directory: string, packages: { id: string; digest: string; enabled: boolean; problem: string }[]) {
  confirmedChoice = settings.theme;
  const packageId = settings.theme.id.split(':')[0];
  const plugin = packages.find(p => p.id === packageId);
  const key = JSON.stringify([directory, settings.theme, plugin && [plugin.digest, plugin.enabled, plugin.problem]]);
  if (root && key !== settingsKey) void chooseTheme(settings.theme, false, !settingsKey);
  settingsKey = key;
}
export function refreshThemeRoot(element: HTMLElement | undefined) {
  if (!element || element === root || disposed) return;
  root = element;
  const token = get(appearance).active?.token;
  if (token) sessions.get(token)?.mount();
}
export function attachTheme(element: HTMLElement, choice: ThemeChoice) {
  root = element; disposed = false; confirmedChoice = choice;
  void chooseTheme(choice, false, true).then(() => { if (!get(appearance).active) void chooseTheme({ ...choice, id: 'builtin:default' }, false); });
  const media = [matchMedia('(prefers-color-scheme: dark)'), matchMedia('(forced-colors: active)')];
  const changed = () => void chooseTheme(confirmedChoice, false);
  for (const query of media) query.addEventListener('change', changed);
  const recover = (event: KeyboardEvent) => { if (event.ctrlKey && event.altKey && event.key.toLowerCase() === 't') { event.preventDefault(); void restoreTheme(); } };
  const wake = () => void poll();
  let polling = false;
  async function poll() {
    if (disposed || polling) return;
    polling = true;
    try {
      const state = await action<ThemeState>('themeState');
      if (state.active && (state.revision !== get(appearance).revision || state.preview)) await render(state);
    } catch (e) { if (!disposed) themeError.set(errorMessage(e)); }
    finally { polling = false; }
  }
  const timer = setInterval(() => void poll(), 500);
  window.addEventListener('keydown', recover, true); window.addEventListener('focus', wake); document.addEventListener('visibilitychange', wake);
  return () => {
    disposed = true; ++requested; root = undefined; clearInterval(timer);
    for (const query of media) query.removeEventListener('change', changed);
    window.removeEventListener('keydown', recover, true); window.removeEventListener('focus', wake); document.removeEventListener('visibilitychange', wake);
    for (const session of sessions.values()) session.dispose(); sessions.clear(); void action('themeCancel').catch(() => {});
    for (const sync of panelSync.values()) sync.resolve(false); panelSync.clear();
  };
}
