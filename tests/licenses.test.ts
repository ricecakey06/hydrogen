import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import test from 'node:test';

test('bundled licenses match the dependency locks and have readable notices', () => {
  const read = (file: string) => readFileSync(new URL(`../${file}`, import.meta.url), 'utf8');
  const { locks, packages, texts } = JSON.parse(read('licenses/third-party.json'));
  assert.deepEqual(Object.keys(locks), ['package-lock.json', 'src-tauri/Cargo.lock', 'official-plugins/Cargo.lock']);
  for (const [file, hash] of Object.entries(locks)) {
    const current = createHash('sha256').update(read(file).replaceAll('\r\n', '\n')).digest('hex');
    assert.equal(hash, current, `${file} changed. Regenerate the notices using licenses/README.md.`);
  }
  for (const item of packages) {
    assert.ok(item.license && item.texts.length, `${item.name} is missing license text`);
    for (const index of item.texts) assert.ok(texts[index]?.trim(), `${item.name} refers to an empty notice`);
  }
});
