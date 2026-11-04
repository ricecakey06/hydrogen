import assert from 'node:assert/strict';
import test from 'node:test';
import { selectionFromBox } from '../official-plugins/ui.shell/frontend/lib/mod-selection.ts';

test('box selection toggles intersected mods against the state at drag start', () => {
  const original = ['selected.jar', 'outside.jar'];
  const hits = ['selected.jar', 'new.jar'];
  assert.deepEqual(selectionFromBox(original, hits), ['outside.jar', 'new.jar']);
  assert.deepEqual(selectionFromBox(original, hits), ['outside.jar', 'new.jar'], 'moving inside the same cards must not toggle them repeatedly');
  assert.deepEqual(selectionFromBox(original, ['new.jar']), [...original, 'new.jar'], 'shrinking the box restores cards that are no longer intersected');
  assert.deepEqual(selectionFromBox(original, []), original, 'mods outside the box retain their selection');
  assert.deepEqual(original, ['selected.jar', 'outside.jar'], 'the drag-start snapshot is immutable');
  assert.deepEqual(selectionFromBox([], ['new.jar', 'new.jar']), ['new.jar']);
});
