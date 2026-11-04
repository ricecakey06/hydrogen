<script lang="ts">
  let { name, size = 24, width = 2 }: { name: string; size?: number; width?: number } = $props();
  const paths: Record<string, string> = {
    panel: 'M3 3h18v18H3ZM9 3v18M16 12h2',
    profiles: 'M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2M13 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0ZM22 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75',
    user: 'M17 6a5 5 0 1 1-10 0 5 5 0 0 1 10 0ZM2 22a10 10 0 0 1 20 0',
    'circle-play': 'M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0ZM9 7l7 5-7 5Z',
    gear: 'M9.66 4.35L10.61 2.10L13.39 2.10L14.34 4.35L15.76 4.94L18.02 4.01L19.99 5.98L19.06 8.24L19.65 9.66L21.90 10.61L21.90 13.39L19.65 14.34L19.06 15.76L19.99 18.02L18.02 19.99L15.76 19.06L14.34 19.65L13.39 21.90L10.61 21.90L9.66 19.65L8.24 19.06L5.98 19.99L4.01 18.02L4.94 15.76L4.35 14.34L2.10 13.39L2.10 10.61L4.35 9.66L4.94 8.24L4.01 5.98L5.98 4.01L8.24 4.94ZM15.5 12a3.5 3.5 0 1 1-7 0 3.5 3.5 0 0 1 7 0Z',
    back: 'm15 5-7 7 7 7', close: 'm5 5 14 14M19 5 5 19', minus: 'M2 12h20',
    spinner: 'M12 2a10 10 0 0 1 10 10',
    stop: 'M5 5h14v14H5Z',
    settings: 'M3 5h18M3 12h18M3 19h18M8 2v6M16 9v6M9 16v6',
    download: 'M12 2v14m-6-6 6 6 6-6M3 18v4h18v-4', refresh: 'M20 9a8 8 0 1 0 0 6M20 3v6h-6',
    puzzle: 'M3 5h6V3a3 3 0 0 1 6 0v2h6v6h2a3 3 0 0 1 0 6h-2v6h-6v-2a3 3 0 0 0-6 0v2H3v-6h2a3 3 0 0 0 0-6H3Z',
    folder: 'M2 6h8l2 3h10v12H2ZM2 9V5h7l2 2', chevron: 'm9 5 7 7-7 7',
    cube: 'm12 2 9 5v10l-9 5-9-5V7Zm0 10 9-5M12 12 3 7m9 5v10',
    play: 'm6 5 12 7-12 7Z', check: 'm4 12 5 5L20 6', plus: 'M12 3v18M3 12h18',
    'circle-check': 'M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0Zm-15 0 3 3 7-7',
    'circle-minus': 'M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0ZM7 12h10',
    trash: 'M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7',
  };
</script>
<svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width={width} stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={paths[name] ?? paths.puzzle} /></svg>
