<!-- Primary: theme selection and live preview; secondary: scheme/material; tertiary: origin and restore. Spacing: 12/16/24px; neutral surfaces and text use the theme tokens. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { action, type Snapshot, errorMessage } from '../lib/api';
  import { appearance, chooseTheme, restoreTheme, type ThemeChoice, type ThemeEntry } from '../lib/theme';
  import { t } from '../lib/i18n';
  let { snapshot, onplugins }: { snapshot: Snapshot | null; onplugins?: (id: string) => void } = $props();
  let entries = $state<ThemeEntry[]>([]);
  let error = $state('');
  let mode = $state<ThemeChoice['mode']>('system');
  let material = $state<ThemeChoice['material']>('auto');
  let busy = $state(false);
  $effect(() => { if (!$appearance.preview && $appearance.active) { mode = $appearance.active.choice.mode; material = $appearance.active.choice.material; } });
  onMount(() => { mode = snapshot?.settings.theme.mode ?? 'system'; material = snapshot?.settings.theme.material ?? 'auto'; void action<ThemeEntry[]>('themeCatalog').then(value => entries = value).catch(e => error = errorMessage(e)); });
  async function select(id: string) { busy = true; try { await chooseTheme({ id, mode, material }); } finally { busy = false; } }
</script>
<section class="appearance-settings" aria-label={t('外观')}>
  <div class="appearance-heading"><div><h2>{t('外观')}</h2><p class="caption">{t('预览完整皮肤，保留后保存。20 秒未确认将自动撤销。')}</p></div><button data-h-part="button" class="text-button" onclick={() => void restoreTheme()}>{t('恢复默认')}</button></div>
  <div class="appearance-options">
    <label>{t('明暗模式')}<select data-h-part="input-shell" bind:value={mode} onchange={() => void select($appearance.active?.choice.id ?? snapshot?.settings.theme.id ?? 'builtin:default')}><option value="system">{t('跟随系统')}</option><option value="light">{t('浅色')}</option><option value="dark">{t('深色')}</option></select></label>
    <label>{t('窗口材质')}<select data-h-part="input-shell" bind:value={material} onchange={() => void select($appearance.active?.choice.id ?? snapshot?.settings.theme.id ?? 'builtin:default')}><option value="auto">{t('自动')}</option><option value="opaque">{t('不透明')}</option></select></label>
  </div>
  <div class="theme-cards">
    {#each entries as entry (entry.id)}
      <div class="theme-card" data-h-part="card">
        <button data-h-part="button" aria-pressed={($appearance.active?.choice.id ?? snapshot?.settings.theme.id) === entry.id} disabled={!entry.available} onclick={() => void select(entry.id)}>
          {#if entry.preview}<img class="theme-preview" src={entry.preview} alt="" loading="lazy" />{:else}<span class="theme-swatch" class:dark={mode === 'dark'} aria-hidden="true"><span>H</span><i></i><i></i></span>{/if}
          <strong>{entry.title}</strong><small>{entry.variants.map(s => s === 'light' ? t('浅色') : t('深色')).join(' / ')}{!entry.available ? ' · ' + t('插件不可用') : ''}</small>
        </button>
        {#if entry.pluginId && onplugins}<button data-h-part="button" class="text-button" onclick={() => onplugins?.(entry.pluginId!)}>{t('插件详情')}</button>{/if}
      </div>
    {/each}
  </div>
  {#if busy}<p class="caption" role="status">{t('正在准备主题…')}</p>{/if}
  <p class="caption" role="status">{t('实际主题')}：{$appearance.active?.actualId ?? 'builtin:default'} · {$appearance.active?.scheme ?? mode}</p>
  {#if $appearance.active?.fallbackReason}<p class="issue">{$appearance.active.fallbackReason}</p>{/if}
  {#if error}<p class="issue" role="alert">{error}</p>{/if}
  <p class="caption">{t('紧急恢复快捷键：Ctrl + Alt + T。也可使用 --safe-theme 启动。')}</p>
</section>
<style>
  .appearance-settings { margin-block: 24px; padding-bottom: 24px; border-bottom: 1px solid var(--h-border); }
  .appearance-heading { display: flex; align-items: start; justify-content: space-between; gap: 16px; }
  h2 { font-size: 18px; font-weight: 500; }
  .appearance-options { display: grid; grid-template-columns: repeat(2,minmax(0,1fr)); gap: 16px; margin-block: 20px; }
  label { font-size: 14px; color: var(--h-text-secondary); } select { margin-top: 8px; }
  .theme-cards { display: grid; grid-template-columns: repeat(auto-fit,minmax(160px,1fr)); gap: 12px; }
  .theme-card > button:first-child { width: 100%; display: flex; flex-direction: column; align-items: stretch; padding: 12px; text-align: left; border: 1px solid var(--h-border); }
  button[aria-pressed=true] { outline: 2px solid var(--h-focus-ring); outline-offset: 2px; }
  small { font-size: 12px; margin-top: 4px; color: var(--h-text-secondary); }
  .theme-swatch { height: 72px; padding: 12px; display: grid; grid-template-columns: 28px 1fr; gap: 8px; margin-bottom: 12px; border-radius: 6px; background: #e8ecf2; color: #333b48; }
  .theme-swatch span { grid-row: span 2; } .theme-swatch i { background: #fff; border-radius: 4px; }
  .theme-swatch.dark { background: #17202c; color: #e5edf7; } .theme-swatch.dark i { background: #3e4e63; }
  .theme-preview { width: 100%; height: 96px; object-fit: cover; border-radius: 6px; margin-bottom: 12px; }
  .theme-card > .text-button { margin: 0 12px 8px; font-size: 12px; }
</style>
