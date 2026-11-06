import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { parse } from 'svelte/compiler';
import ts from 'typescript';

const source = readFileSync(new URL('../official-plugins/ui.launcher/frontend/pages/Resources.svelte', import.meta.url), 'utf8');
const tree = parse(source, { modern: true }) as any;
const elements: any[] = [];
function visit(node: any) {
  if (!node || typeof node !== 'object') return;
  if (node.type === 'RegularElement') elements.push(node);
  for (const value of Object.values(node)) {
    if (Array.isArray(value)) value.forEach(visit);
    else if (value && typeof value === 'object') visit(value);
  }
}
visit(tree.fragment);
const attribute = (element: any, name: string) => element.attributes.find((item: any) => item.name === name)?.value;
const input = elements.find(element => element.name === 'input' && attribute(element, 'type')?.[0]?.data === 'search');
const form = elements.find(element => element.name === 'form' && attribute(element, 'role')?.[0]?.data === 'search');
const handler = (element: any, name: string) => {
  const expression = attribute(element, name)?.expression;
  return expression ? source.slice(expression.start, expression.end) : '() => {}';
};
const functions = tree.instance.content.body.filter((node: any) => node.type === 'FunctionDeclaration'
  && ['refresh', 'search', 'changeFilters', 'changeSource', 'loadFilters'].includes(node.id.name))
  .map((node: any) => source.slice(node.start, node.end)).join('\n');

// Run the component's own handlers and request sequencing with browser input/form and timer doubles.
// This covers IMEs that omit a final non-composing input event, without copying the search implementation.
function catalog() {
  let timer: (() => void) | undefined;
  class Input {
    currentTarget: { value: string };
    isComposing: boolean;
    constructor(currentTarget: { value: string }, isComposing = false) { this.currentTarget = currentTarget; this.isComposing = isComposing; }
  }
  class Form {
    input: { value: string };
    constructor(input: { value: string }) { this.input = input; }
    get(name: string) { return name === attribute(input, 'name')?.[0]?.data ? this.input.value : null; }
  }
  const code = ts.transpileModule(`
    const emptyQuery = { query: '', gameVersion: '', category: '', loader: '', source: 'minecraft.resources:modrinth' };
    let query = { ...emptyQuery }, applied = emptyQuery, results = { hits: [], total_hits: 0 }, offset = 0;
    let searchSequence = 0, searchTimer, searching = false, searchError = '', lastSearch, list;
    let filterSequence = 0, loadingFilters = false, filtersError = false, filters = {};
    const inputDelay = 300, requests = [];
    const errorMessage = error => error.message;
    const action = (type, criteria) => type === 'resourceFilters'
      ? Promise.resolve({ categories: [], gameVersions: [] })
      : new Promise((resolve, reject) => requests.push({ criteria, resolve, reject }));
    ${functions}
    return {
      input: ${handler(input, 'oninput')}, compositionEnd: ${handler(input, 'oncompositionend')},
      submit: ${handler(form, 'onsubmit')}, changeSource, refresh, requests,
      get state() { return { query, applied, results, offset, searching, searchError }; }
    };
  `, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const instance = new Function('InputEvent', 'FormData', 'setTimeout', 'clearTimeout', code)(
    Input, Form, (callback: () => void) => { timer = callback; return 1; }, () => { timer = undefined; },
  );
  const visible = { value: '' };
  return {
    ...instance,
    state: () => instance.state,
    type(value: string, composing = false) { visible.value = value; instance.input(new Input(visible, composing)); },
    commit() { instance.compositionEnd({ currentTarget: visible }); },
    submit(value = visible.value) { visible.value = value; instance.submit({ currentTarget: visible, preventDefault() {} }); },
    timer() { const callback = timer; timer = undefined; callback?.(); },
    async finish(index: number, title: string) {
      instance.requests[index].resolve({ hits: [{ title }], total_hits: 1 });
      await Promise.resolve();
    },
  };
}

test('IME confirmation and form submission use visible text even without a final input event', async () => {
  const page = catalog();
  page.type('钠', true);
  assert.equal(page.requests.length, 0, 'composition must not start an automatic search');
  page.commit();
  page.submit();
  assert.equal(page.requests[0].criteria.query, '钠');
  page.timer();
  assert.equal(page.requests.length, 1, 'submission must clear the pending automatic search');
  await page.finish(0, '钠');
  assert.equal(page.state().results.hits[0].title, '钠');

  page.submit('Sodium'); // Browser autofill can change the visible value without an input event.
  assert.equal(page.requests[1].criteria.query, 'Sodium');
  await page.finish(1, 'Sodium');
  assert.equal(page.state().results.hits[0].title, 'Sodium');
});

test('automatic IME search, immediate submit and source changes retain the latest criteria', () => {
  const page = catalog();
  page.type('钠', true);
  page.commit();
  page.timer();
  assert.equal(page.requests[0].criteria.query, '钠');
  page.type('Sodium');
  page.submit();
  assert.equal(page.requests[1].criteria.query, 'Sodium');
  page.changeSource('minecraft.resources:curseforge');
  assert.equal(page.requests[2].criteria.query, 'Sodium');
  assert.equal(page.requests[2].criteria.source, 'minecraft.resources:curseforge');
});

test('late search responses cannot overwrite newer results and failed searches can be retried', async () => {
  const page = catalog();
  page.type('old');
  page.timer();
  page.type('new');
  page.submit();
  await page.finish(1, 'new');
  await page.finish(0, 'old');
  assert.equal(page.state().results.hits[0].title, 'new');
  assert.equal(page.state().searching, false);
  page.submit('retry');
  page.requests[2].reject(new Error('Network failed'));
  await Promise.resolve();
  assert.equal(page.state().searchError, 'Network failed');
  page.submit();
  await page.finish(3, 'retry');
  assert.equal(page.state().searchError, '');
  assert.equal(page.state().results.hits[0].title, 'retry');
});
