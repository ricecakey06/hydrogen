import { writable, get } from 'svelte/store';
import { invoke } from '@tauri-apps/api/core';

export interface Language { locale: string; name: string }
export interface LanguageBundle { locale: string; name: string; minecraftLanguage: string; messages: Record<string, string> }
const fallback: LanguageBundle & { languages: Language[] } = { locale: 'zh-CN', name: '简体中文', minecraftLanguage: 'zh_cn', messages: {}, languages: [{ locale: 'zh-CN', name: '简体中文' }, { locale: 'en-US', name: 'English' }] };
export const i18n = writable(fallback);
let patterns: { match: RegExp; names: string[]; value: string }[] = [];
const placeholder = /\{([\w.:]+)\}/g;
const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
export function setLanguage(bundle: LanguageBundle, languages: Language[]) {
  patterns = Object.entries(bundle.messages).filter(([key]) => /\{[\w.:]+\}/.test(key)).map(([key, value]) => {
    const names: string[] = []; let start = 0; let pattern = '^';
    for (const match of key.matchAll(placeholder)) { pattern += escape(key.slice(start, match.index)) + '(.+?)'; names.push(match[1]); start = match.index! + match[0].length; }
    return { match: new RegExp(pattern + escape(key.slice(start)) + '$', 's'), names, value };
  });
  i18n.set({ ...bundle, languages });
  if (typeof document !== 'undefined') document.documentElement.lang = bundle.locale;
}
export function t(source: string | null | undefined, ...values: unknown[]): string {
  if (source == null) return '';
  const bundle = get(i18n);
  const direct = Object.hasOwn(bundle.messages, source) ? bundle.messages[source] : undefined;
  if (direct !== undefined || values.length) return (direct ?? source).replace(placeholder, (token, key) => values[Number(key)] == null ? token : String(values[Number(key)]));
  if (bundle.locale === 'zh-CN') return source;
  for (const pattern of patterns) {
    const match = pattern.match.exec(source);
    if (match) return pattern.value.replace(placeholder, (token, key) => { const index = pattern.names.indexOf(key); return index < 0 ? token : match[index + 1]; });
  }
  return source;
}
let loaded = ''; let pending: Promise<void> = Promise.resolve();
export function loadLanguage(root: string, locale: string, plugin?: { enabled: boolean; digest: string; problem: string }) {
  const key = `${root}/${locale}/${plugin?.enabled}/${plugin?.digest}/${plugin?.problem}`;
  if (key === loaded) return pending;
  loaded = key;
  pending = (async () => {
    if (!plugin?.enabled || plugin.problem) { setLanguage(fallback, fallback.languages); return; }
    try {
      const result = await invoke<{ bundle: LanguageBundle; languages: Language[] }>('launcher_action', { action: { type: 'i18nBundle', locale } });
      if (key === loaded) setLanguage(result.bundle, result.languages);
    } catch { if (key === loaded) setLanguage(fallback, fallback.languages); }
  })();
  return pending;
}
