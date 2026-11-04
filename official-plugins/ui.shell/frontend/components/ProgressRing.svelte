<script lang="ts">
  let { progress = null, size = 114 }: { progress?: number | null; size?: number } = $props();
</script>
<svg class="progress-ring" class:indeterminate={progress === null} width={size} height={size} viewBox="0 0 100 100" fill="none" aria-hidden="true"><circle cx="50" cy="50" r="47" pathLength="100" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" stroke-dasharray={progress === null ? '25 75' : `${Math.max(0, Math.min(1, progress)) * 100} 100`} /></svg>
<style>
  /* The decorative overlay must let clicks reach the launch/cancel button beneath it. */
  .progress-ring { transform: rotate(-90deg); pointer-events: none; }
  @media (prefers-reduced-motion: no-preference) { circle { transition: stroke-dasharray .18s cubic-bezier(.333333,1,.666667,1); } .indeterminate { animation: ring-spin 1.2s linear infinite; } @keyframes ring-spin { to { transform: rotate(270deg); } } }
</style>
