import assert from 'node:assert/strict';
import test from 'node:test';
import { animateText, entrance, fadeSlide } from '../official-plugins/ui.shell/frontend/lib/motion.ts';

test('Svelte default transition parameters produce finite timing and reduced motion skips delays', t => {
  const media = { matches: false };
  const originalMedia = globalThis.matchMedia;
  const originalStyle = globalThis.getComputedStyle;
  globalThis.matchMedia = (() => media) as typeof matchMedia;
  globalThis.getComputedStyle = (() => ({ opacity: '1', transform: 'none' })) as typeof getComputedStyle;
  t.after(() => { globalThis.matchMedia = originalMedia; globalThis.getComputedStyle = originalStyle; });
  const node = {} as Element;
  assert.ok(Number.isFinite(entrance(node, {}).delay), 'Svelte passes an empty object when no params are given');
  media.matches = true;
  assert.equal(entrance(node, { delay: 140 }).delay, 0);
  assert.equal(entrance(node, {}).duration, 0);
  for (const direction of ['in', 'out'] as const) {
    assert.equal(fadeSlide(node, {}, { direction }).duration, 0);
    assert.equal(fadeSlide(node, { compact: true }, { direction }).duration, 0);
  }
});

test('text updates remain readable, cancel replaced effects, and respect reduced motion', t => {
  let now = 0;
  const media = { matches: false };
  const originalMedia = globalThis.matchMedia;
  globalThis.matchMedia = (() => media) as typeof matchMedia;
  t.after(() => { globalThis.matchMedia = originalMedia; });
  t.mock.method(performance, 'now', () => now);
  const effects: { cancelled: boolean }[] = [];
  const node = { animate() {
    const effect = { cancelled: false, cancel() { this.cancelled = true; } };
    effects.push(effect);
    return effect;
  } } as unknown as HTMLElement;

  const text = animateText(node, 'ready');
  text.update('ready');
  assert.equal(effects.length, 0, 'unchanged text must not flash');
  text.update('1%');
  assert.equal(effects.length, 1);
  now = 100;
  text.update('2%');
  assert.equal(effects.length, 1, 'rapid progress must not repeatedly fade');
  assert.equal(effects[0].cancelled, true);
  now = 400;
  text.update('done');
  assert.equal(effects.length, 2, 'an update after a quiet interval can animate');
  text.destroy();
  assert.equal(effects[1].cancelled, true, 'unmount must release the active effect');

  media.matches = true;
  const reduced = animateText(node, 'ready');
  now = 1000;
  reduced.update('done');
  assert.equal(effects.length, 2, 'reduced motion must not start an effect');
  reduced.destroy();
});
