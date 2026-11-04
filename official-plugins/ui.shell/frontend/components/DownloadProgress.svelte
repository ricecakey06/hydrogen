<script lang="ts">
  let { percent, indeterminate = false, overall = false }: { percent: number; indeterminate?: boolean; overall?: boolean } = $props();
</script>
<div class="download-progress" class:indeterminate class:overall role="progressbar" aria-label={overall ? '总体下载进度' : '下载进度'} aria-valuemin={0} aria-valuemax={100} aria-valuenow={indeterminate ? undefined : Math.round(percent)}>
  <span style:width={indeterminate ? '30%' : `${percent}%`}></span>
</div>
