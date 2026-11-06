<script lang="ts">
  import { appearance, themeError, confirmTheme, cancelTheme, restoreTheme } from '../lib/theme';
  import { t } from '../lib/i18n';
</script>
{#if $appearance.preview || $themeError}
  <aside class="theme-recovery" aria-live="polite" aria-label={t('主题恢复')}>
    {#if $appearance.preview}
      <span>{t('保留此外观？')} {$appearance.remainingSeconds}s</span>
      <button disabled={$appearance.status === 'syncing'} onclick={() => void confirmTheme()}>{t('保留')}</button>
      <button onclick={() => void cancelTheme()}>{t('撤销')}</button>
    {/if}
    {#if $themeError}<span role="alert">{$themeError}</span><button onclick={() => void restoreTheme()}>{t('恢复默认主题')}</button>{/if}
  </aside>
{/if}
<style>
  .theme-recovery { all: initial; position: fixed; z-index: 2147483647; bottom: 16px; left: 50%; transform: translateX(-50%); display: flex; flex-wrap: wrap; align-items: center; gap: 10px; max-width: calc(100vw - 32px); padding: 12px 16px; background: #fff; color: #18202b; border: 2px solid #536275; border-radius: 12px; box-shadow: 0 4px 24px #0004; font: 14px/1.5 'Microsoft YaHei UI', sans-serif; }
  .theme-recovery button { all: initial; cursor: pointer; padding: 7px 12px; border: 1px solid #536275; border-radius: 6px; color: #18202b; background: #edf1f6; font: inherit; white-space: nowrap; }
  .theme-recovery button:focus-visible { outline: 3px solid #245aa0; outline-offset: 2px; }
  @media (forced-colors: active) { .theme-recovery { background: Canvas; color: CanvasText; border-color: ButtonText; } .theme-recovery button { background: ButtonFace; color: ButtonText; border-color: ButtonText; } }
</style>
