import assert from 'node:assert/strict';
import test from 'node:test';
import { instanceIconKind, instanceIconSources } from '../official-plugins/ui.launcher/frontend/lib/instance-icons.ts';

test('created and discovered instances select their loader artwork, including legacy Fabric records', () => {
  for (const kind of ['forge', 'neoforge', 'fabric'] as const) {
    assert.equal(instanceIconKind({ components: { kind, loader: '1', fabricApi: null }, loaderName: 'Vanilla' }), kind);
    assert.equal(instanceIconKind({ components: null, loaderName: kind.toUpperCase() + ' 1.21.1' }), kind);
  }
  assert.equal(instanceIconKind({ components: { loader: '0.16', fabricApi: null }, loaderName: '' }), 'fabric');
  assert.equal(instanceIconKind({ components: { kind: '', loader: '0.16', fabricApi: null }, loaderName: '' }), 'fabric');
  for (const loaderName of ['', '原版', 'ForgeLike', 'example:forge', 'NeoForgeExtra', 'Quilt']) {
    assert.equal(instanceIconKind({ components: null, loaderName }), 'vanilla');
  }
  assert.equal(instanceIconKind({ components: null, loaderName: ' NeoForge-21.1 ' }), 'neoforge');
  assert.equal(instanceIconKind({ components: { kind: 'example:loader', loader: '1', fabricApi: null }, loaderName: 'Forge' }), 'vanilla');
});

test('pack artwork wins over theme overrides and builtins remain the final fallback', () => {
  assert.deepEqual(instanceIconSources('pack', 'theme', 'builtin'), ['pack', 'theme', 'builtin']);
  assert.deepEqual(instanceIconSources(undefined, 'theme', 'builtin'), ['theme', 'builtin']);
  assert.deepEqual(instanceIconSources(undefined, undefined, 'builtin'), ['builtin']);
  assert.deepEqual(instanceIconSources('', 'builtin', 'builtin'), ['builtin']);
});
