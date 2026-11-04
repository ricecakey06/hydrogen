import assert from 'node:assert/strict';
import test from 'node:test';
import { legacyHatIsVisible, skinLayout } from '../official-plugins/ui.shell/frontend/lib/avatar.ts';

test('modern, legacy and high resolution skin layouts use the correct head coordinates', () => {
  assert.deepEqual(skinLayout(64,64), { scale:1, legacy:false });
  assert.deepEqual(skinLayout(64,32), { scale:1, legacy:true });
  assert.deepEqual(skinLayout(128,128), { scale:2, legacy:false });
  assert.deepEqual(skinLayout(128,64), { scale:2, legacy:true });
  for (const [width,height] of [[0,0],[32,32],[65,65],[64,16],[2048,2048]]) assert.throws(() => skinLayout(width,height));
});

test('fully opaque legacy hat pixels cannot cover the real face', () => {
  const rgba = new Uint8ClampedArray(32*32*4).fill(255);
  assert.equal(legacyHatIsVisible(rgba), false);
  rgba[3] = 128;
  assert.equal(legacyHatIsVisible(rgba), false);
  rgba[3] = 0;
  assert.equal(legacyHatIsVisible(rgba), true);
});
