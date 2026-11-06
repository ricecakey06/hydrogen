import assert from 'node:assert/strict';
import test from 'node:test';
import { mockIPC } from '@tauri-apps/api/mocks';
import { get } from 'svelte/store';
import { VisualSession, refreshThemeRoot, appearance, themeError, timeout, type ThemeVisual } from '../official-plugins/ui.shell/frontend/lib/theme.ts';

// A deterministic DOM boundary: no network, native calls, or third-party test runtime.
class Node {
  dataset: Record<string,string> = {};
  children: Node[] = [];
  parent?: Node;
  hidden = false; src = ''; media = ''; rel = ''; href = ''; alt = ''; draggable = false;
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  append(...nodes: Node[]) { for (const node of nodes) { node.remove(); node.parent = this; this.children.push(node); } }
  remove() { if (this.parent) this.parent.children = this.parent.children.filter(node => node !== this); this.parent = undefined; }
  setAttribute() {}
  removeAttribute(name: string) { if (name === 'src') this.src = ''; }
}

function environment(t: test.TestContext, loadStyles = true, decode: () => Promise<void> = () => Promise.resolve()) {
  const previous = { document: globalThis.document, window: globalThis.window, Image: globalThis.Image, FontFace: globalThis.FontFace };
  const head = new Node(), slot = new Node(), logo = new Node(), brand = new Node();
  const root = new Node();
  (root as any).querySelector = (selector: string) => selector.includes('theme-slot') ? slot : selector.includes('brand-image') ? logo : brand;
  const append = head.append.bind(head);
  head.append = (...nodes) => { append(...nodes); if (loadStyles) for (const node of nodes) queueMicrotask(() => node.onload?.()); };
  globalThis.document = { head, createElement: () => new Node(), fonts: { load: () => Promise.resolve([]) } } as any;
  globalThis.window = {} as any;
  globalThis.Image = class extends Node { decode() { return decode(); } } as any;
  globalThis.FontFace = class { load() { return Promise.resolve(this); } } as any;
  const released: string[] = [];
  mockIPC((command, args: any) => { assert.equal(command, 'launcher_action'); if (args.action.type === 'themeRelease') released.push(args.action.token); return null; });
  refreshThemeRoot(root as any);
  t.after(() => Object.assign(globalThis, previous));
  return { head, slot, root, logo, brand, released };
}
const visual = (token: string): ThemeVisual => ({ token, choice: { id: 'builtin:default', mode: 'light', material: 'auto' }, actualId: 'builtin:default', scheme: 'light', highContrast: false, packageDigest: null, fallbackReason: null, styles: ['main.css'], logo: 'logo.png', layers: [{ id: 'figure', slot: 'content.background', source: 'figure.png' }], images: ['figure.png'], fonts: [], fontSources: [], bytes: 0, decodedBytes: 0 });

test('100 visual sessions leave no owned styles/layers and release exactly once', async t => {
  const dom = environment(t);
  const unrelated = new Node(); dom.head.append(unrelated);
  for (let i = 0; i < 100; i++) {
    const session = new VisualSession(visual(`lease-${i}`));
    await Promise.all([session.prepare(), session.prepare()]);
    assert.equal(session.links.length, 1, 'concurrent callers share preparation');
    session.mount(); assert.equal(dom.slot.children.length, 1);
    assert.equal(dom.root.dataset.hSession, `lease-${i}`);
    session.unmount(); session.mount(); session.dispose(); session.dispose();
    assert.deepEqual(dom.head.children, [unrelated]); assert.equal(dom.slot.children.length, 0);
    assert.ok(session.loadingImages.every(image => !image.src));
  }
  await Promise.resolve();
  assert.equal(dom.released.length, 100); assert.equal(new Set(dom.released).size, 100);
});

test('an aborted image load cannot insert late layers or styles', async t => {
  let complete!: () => void;
  const dom = environment(t, true, () => new Promise(resolve => { complete = resolve; }));
  const session = new VisualSession(visual('late'));
  const preparing = session.prepare();
  const rejected = assert.rejects(preparing, /THEME_STALE/);
  session.dispose(); complete(); await rejected; await Promise.resolve();
  assert.equal(session.nodes.length, 0); assert.equal(dom.head.children.length, 0);
  assert.throws(() => session.mount(), /THEME_STALE/);
});

test('stylesheet cancellation clears callbacks and cannot remount on late completion', async t => {
  const dom = environment(t, false);
  const session = new VisualSession(visual('late-style'));
  const preparation = session.prepare(), complete = session.links[0].onload;
  const rejected = assert.rejects(preparation, /THEME_STALE/);
  session.dispose(); complete?.(); await rejected;
  assert.equal(session.links[0].onload, null); assert.equal(dom.head.children.length, 0);
});

test('timeouts and already-aborted loads terminate without waiting for the resource', async () => {
  await assert.rejects(timeout(new Promise(() => {}), 5), /THEME_SYNC_TIMEOUT/);
  const controller = new AbortController(); controller.abort();
  await assert.rejects(timeout(new Promise(() => {}), 1000, controller.signal), /THEME_STALE/);
});

test('missing slots fail before replacing the active root and cleanup continues after a failure', async t => {
  const dom = environment(t);
  const session = new VisualSession(visual('broken'));
  await session.prepare(); dom.root.dataset.hSession = 'confirmed';
  (dom.root as any).querySelector = () => null;
  assert.throws(() => session.mount(), /THEME_PART/); assert.equal(dom.root.dataset.hSession, 'confirmed');
  session.nodes[0].remove = () => { throw new Error('fixture cleanup failure'); };
  session.dispose(); await Promise.resolve();
  assert.equal(dom.head.children.length, 0, 'remaining cleanup runs');
  assert.equal(get(appearance).status, 'cleanup-incomplete'); assert.match(get(themeError), /THEME_CLEANUP_INCOMPLETE/);
});
