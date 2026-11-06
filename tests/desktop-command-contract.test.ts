import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

test('trusted desktop commands are declared and granted only to the main window', () => {
  const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8');
  const desktop = read('../src-tauri/src/desktop.rs');
  const build = read('../src-tauri/build.rs');
  const registered = desktop.match(/generate_handler!\[([\s\S]*?)\]/)?.[1]
    .split(',').map(command => command.trim()).filter(Boolean).sort();
  const declared = Array.from(build.match(/\.commands\(&\[([\s\S]*?)\]/)?.[1]
    .matchAll(/"([a-z_]+)"/g) ?? [], match => match[1]).sort();
  assert.ok(registered?.length, 'desktop command handler must be present');
  assert.deepEqual(declared, registered, 'registered commands need generated Tauri ACL entries');

  const capability = JSON.parse(read('../src-tauri/capabilities/main.json'));
  assert.deepEqual(capability.windows, ['main']);
  assert.equal(capability.local, true);
  assert.equal(capability.remote, undefined, 'isolated panels must not receive application IPC');
  for (const command of registered) {
    assert.ok(capability.permissions.includes(`allow-${command.replaceAll('_', '-')}`),
      `${command} must be callable by the trusted main document`);
  }
});
