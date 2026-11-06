import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile, readdir } from 'node:fs/promises';
import { parse } from 'svelte/compiler';
import { setLanguage, t, type LanguageBundle } from '../official-plugins/ui.shell/frontend/lib/i18n.ts';

const base = new URL('../official-plugins/i18n/assets/locales/', import.meta.url);
const chinese: LanguageBundle = JSON.parse(await readFile(new URL('zh-CN.json', base), 'utf8'));
const english: LanguageBundle = JSON.parse(await readFile(new URL('en-US.json', base), 'utf8'));
test('bundles cover launcher messages, preserve placeholders and format native progress', async () => {
  assert.deepEqual(Object.keys(chinese.messages).sort(), Object.keys(english.messages).sort());
  const placeholders = (s: string) => [...s.matchAll(/\{([\w.:]+)\}/g)].map(m => m[1]).sort();
  for (const [key, value] of Object.entries(english.messages)) {
    assert.ok(value.trim(), key);
    assert.deepEqual(placeholders(value), placeholders(key), key);
  }
  async function check(directory: URL) {
    for (const file of await readdir(directory, { withFileTypes: true })) {
      const url = new URL(file.name + (file.isDirectory() ? '/' : ''), directory);
      if (file.isDirectory()) { await check(url); continue; }
      if (!file.name.endsWith('.svelte')) continue;
      const tree = parse(await readFile(url, 'utf8'), { modern: true });
      function walk(node: unknown) {
        if (!node || typeof node !== 'object') return;
        const n = node as Record<string, any>;
        if (n.type === 'CallExpression' && n.callee?.name === 't' && n.arguments[0]?.type === 'Literal') {
          assert.ok(Object.hasOwn(english.messages, n.arguments[0].value), `${file.name}: ${n.arguments[0].value}`);
        }
        if (n.type === 'Text') assert.ok(!/[\u3400-\u9fff]/.test(n.data), `${file.name}: untranslated text ${n.data}`);
        for (const value of Object.values(n)) if (Array.isArray(value)) value.forEach(walk); else if (value && typeof value === 'object') walk(value);
      }
      walk(tree);
    }
  }
  for (const plugin of ['ui.shell', 'ui.launcher', 'plugins.manager']) await check(new URL(`../official-plugins/${plugin}/frontend/`, import.meta.url));
  setLanguage(english, []);
  assert.equal(t('启动游戏'), 'Launch game');
  assert.equal(t('已添加 {0}', 'Alex'), 'Added Alex');
  assert.equal(t('找到 3 个 Java 运行时'), 'Found 3 Java runtimes');
  assert.equal(t('游戏异常退出（退出码 1）'), 'Game exited unexpectedly (exit code 1)');
  assert.equal(t('正在获取 CurseForge 整合包文件信息（0/6 批）…'), 'Retrieving CurseForge modpack file metadata (batch 0/6)…');
  assert.equal(t('unknown custom text'), 'unknown custom text');
  assert.equal(t('toString'), 'toString');
  setLanguage(chinese, []);
  assert.equal(t('已添加 {0}', 'Alex'), '已添加 Alex');
});
