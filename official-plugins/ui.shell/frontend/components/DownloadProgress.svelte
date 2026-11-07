<script lang="ts">
  import { t } from '../lib/i18n';
  let { percent, indeterminate = false, overall = false, label }: { percent: number; indeterminate?: boolean; overall?: boolean; label?: string } = $props();
</script>
<div class="download-progress" class:indeterminate class:overall role="progressbar" aria-label={label ?? (overall ? t("总体下载进度") : t("下载进度"))} aria-valuemin={0} aria-valuemax={100} aria-valuenow={indeterminate ? undefined : Math.round(percent)}>
  <span style:width={indeterminate ? '30%' : `${percent}%`}></span>
</div>
