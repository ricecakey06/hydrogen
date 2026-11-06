<!-- §2.1 / §3.6 / §6.2: function rows are primary, plugin groups secondary, counts tertiary. Use host popup/text/focus tokens and 8/12/16/24px spacing. -->
<script lang="ts">
  import { tick } from 'svelte';
  import { t } from '../lib/i18n';
  import { invoke, errorMessage, type Snapshot, type RunReport } from '../lib/api';
  import { contributions, uiState, type Contribution, type ViewContext } from '../lib/contributions';
  import PluginPanel from './PluginPanel.svelte';
  import Icon from './Icon.svelte';
  import { sidebarVisibleCount } from '../lib/sidebar-overflow';
  let { snapshot, location, context = {}, onnavigate, onchange, ondone, menu = false, activeKey = '' }: {
    snapshot: Snapshot | null; location: string; context?: ViewContext;
    onnavigate?: (item: Contribution) => void; onchange?: () => Promise<void>;
    ondone?: () => void; menu?: boolean; activeKey?: string;
  } = $props();
  const items = $derived(contributions(snapshot, location, context, $uiState));
  const sidebar = $derived(location.startsWith('sidebar.'));
  const slotId = $props.id();
  let visibleCount = $state(3);
  const visibleItems = $derived(sidebar ? items.slice(0, visibleCount) : items);
  const overflowItems = $derived(sidebar ? items.slice(visibleCount) : []);
  const overflowGroups = $derived.by(() => {
    const groups = new Map<string, { id: string; name: string; items: Contribution[] }>();
    for (const item of overflowItems) {
      if (!groups.has(item.pluginId)) groups.set(item.pluginId, { id: item.pluginId, name: snapshot?.plugins.find(plugin => plugin.id === item.pluginId)?.name ?? item.title, items: [] });
      groups.get(item.pluginId)!.items.push(item);
    }
    return [...groups.values()];
  });
  const overflowActive = $derived(overflowItems.some(item => item.key === activeKey));
  let moreOpen = $state(false);
  let moreButton = $state<HTMLButtonElement>();
  let moreMenu = $state<HTMLDivElement>();
  let refreshLayout = () => {};
  function positionMenu() {
    if (!moreButton || !moreMenu) return;
    // Native top-layer handles clipping; copy root colors so sidebar overrides cannot tint the popup.
    const root = moreMenu.closest('[data-h-theme-root]');
    if (root) {
      const css = getComputedStyle(root);
      for (const token of ['surface-popup', 'surface-card', 'text-primary', 'text-secondary', 'border', 'selection', 'control-hover', 'control-active', 'focus-ring']) {
        moreMenu.style.setProperty(`--h-${token}`, css.getPropertyValue(`--h-${token}`));
      }
      moreMenu.style.colorScheme = css.colorScheme;
    }
    const button = moreButton.getBoundingClientRect();
    const width = Math.min(320, window.innerWidth - 24), height = Math.min(420, window.innerHeight - 24);
    Object.assign(moreMenu.style, { width: `${width}px`, maxHeight: `${height}px` });
    const menuHeight = moreMenu.getBoundingClientRect().height || height;
    Object.assign(moreMenu.style, { left: `${Math.max(12, Math.min(button.right + 12, window.innerWidth - width - 12))}px`, top: `${Math.max(12, Math.min(button.top, window.innerHeight - menuHeight - 12))}px` });
  }
  function measureSidebar(node: HTMLDivElement) {
    if (!sidebar) return;
    const nav = node.closest<HTMLElement>('.sidebar');
    if (!nav) return;
    const outer = [...nav.children].find(child => child.contains(node)) as HTMLElement;
    // The footer's auto top margin is free flex space, not occupied capacity.
    const margin = (element: Element) => {
      const css = getComputedStyle(element);
      return (element.classList.contains('sidebar-footer') ? 0 : parseFloat(css.marginTop) || 0) + (parseFloat(css.marginBottom) || 0);
    };
    const measure = () => {
      if (!node.isConnected) return;
      const css = getComputedStyle(nav), own = getComputedStyle(node);
      const siblings = [...nav.children].filter(child => !child.contains(node) && !child.hasAttribute('data-h-theme-slot'));
      const extra = outer === node ? 0 : outer.getBoundingClientRect().height - node.getBoundingClientRect().height;
      const available = nav.clientHeight - parseFloat(css.paddingTop) - parseFloat(css.paddingBottom)
        - siblings.reduce((height, child) => height + child.getBoundingClientRect().height + margin(child), 0)
        - parseFloat(css.rowGap) * siblings.length - extra - margin(outer);
      const row = node.querySelector(':scope > button')?.getBoundingClientRect().height || nav.querySelector('button')?.getBoundingClientRect().height || 48;
      visibleCount = sidebarVisibleCount(items.length, available, row, parseFloat(own.rowGap) || 8, location === 'sidebar.footer' ? 1 : 3);
      if (moreOpen) positionMenu();
    };
    refreshLayout = measure;
    const observer = new ResizeObserver(measure);
    observer.observe(nav);
    for (const child of nav.children) if (!child.hasAttribute('data-h-theme-slot')) observer.observe(child);
    observer.observe(node);
    measure();
    return { destroy() { observer.disconnect(); refreshLayout = () => {}; } };
  }
  $effect(() => { snapshot; items; void tick().then(() => refreshLayout()); });
  $effect(() => { if (!overflowItems.length && moreOpen) { moreMenu?.hidePopover(); moreOpen = false; } });
  function closeMore() { moreMenu?.hidePopover(); }
  function menuKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); closeMore(); moreButton?.focus(); return; }
    const buttons = [...(moreMenu?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)') ?? [])];
    if (!buttons.length) return;
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    let next: number;
    if (event.key === 'ArrowDown') next = (index + 1) % buttons.length;
    else if (event.key === 'ArrowUp') next = index <= 0 ? buttons.length - 1 : index - 1;
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = buttons.length - 1;
    else return;
    event.preventDefault(); event.stopPropagation(); buttons[next].focus();
  }
  function menuToggled(event: ToggleEvent) {
    moreOpen = event.newState === 'open';
    if (moreOpen) positionMenu();
    if (moreOpen) (moreMenu?.querySelector<HTMLButtonElement>('[role="menuitem"].selected:not(:disabled)') ?? moreMenu?.querySelector<HTMLButtonElement>('[role="menuitem"]:not(:disabled)'))?.focus();
  }
  let opened = $state(''), busy = $state(''), error = $state('');
  async function execute(item: Contribution) {
    closeMore();
    if (item.panel) { if (onnavigate) onnavigate(item); else opened = opened === item.key ? '' : item.key; return; }
    busy = item.key; error = '';
    try {
      const report = await invoke<RunReport>('plugin_run', { pluginId: item.pluginId, command: item.command, params: context });
      if (report.notifications.length) error = report.notifications.join(' · ');
      await onchange?.();
      ondone?.();
    } catch (e) { error = errorMessage(e); }
    finally { busy = ''; }
  }
</script>
{#if items.length}
  <div use:measureSidebar class="plugin-contributions" class:sidebar-contributions={sidebar} class:navigation={!!onnavigate || menu} role={menu ? 'menu' : undefined} aria-label={t("插件扩展")}>
    {#each visibleItems as item (item.key)}
      {#if item.panel && location === 'home.widgets'}
        <section class="plugin-view"><h2>{item.title}</h2><PluginPanel pluginId={item.pluginId} panelId={item.panel} {context} locale={snapshot?.settings.locale} height={220} onerror={message => error = message} /></section>
      {:else}
      <button data-h-part={sidebar ? 'nav-item' : 'button'} class:active={sidebar && item.key === activeKey} aria-current={sidebar && item.key === activeKey ? 'page' : undefined} role={menu ? 'menuitem' : undefined} disabled={!item.enabled || busy === item.key} title={item.title} aria-label={item.title} onclick={() => execute(item)} aria-expanded={item.panel && !onnavigate ? opened === item.key : undefined}>
        {#if sidebar}<Icon name="puzzle" />{/if}<span class:nav-label={sidebar}>{item.title}</span>{#if item.badge}<small class:nav-label={sidebar}>{item.badge}</small>{/if}
      </button>
      {#if item.panel && opened === item.key && !onnavigate}
        <div class="plugin-view"><PluginPanel pluginId={item.pluginId} panelId={item.panel} {context} locale={snapshot?.settings.locale} onerror={message => error = message} /></div>
      {/if}
      {/if}
    {/each}
    {#if overflowItems.length}
      <button bind:this={moreButton} data-h-part="nav-item" class="more-trigger" class:active={moreOpen || overflowActive} popovertarget={`${slotId}-more`} aria-haspopup="menu" aria-expanded={moreOpen} aria-controls={`${slotId}-more`} aria-label={t('更多功能（{0} 项）', overflowItems.length)} title={t('更多功能（{0} 项）', overflowItems.length)}>
        <Icon name="more" /><span class="nav-label">{t('更多')}</span><small class="nav-label more-count">{overflowItems.length}</small>
      </button>
      <div bind:this={moreMenu} id={`${slotId}-more`} popover="auto" class="sidebar-overflow-menu" data-h-scope="popup" role="menu" tabindex="-1" aria-label={t('更多插件功能')} onbeforetoggle={event => { if (event.newState === 'open') positionMenu(); }} ontoggle={menuToggled} onkeydown={menuKeydown}>
        <div class="more-heading"><strong>{t('更多功能')}</strong><span>{t('{0} 项功能', overflowItems.length)}</span></div>
        {#each overflowGroups as group (group.id)}
          <section class="more-group" aria-label={group.name}><h3>{group.name}</h3>
            {#each group.items as item (item.key)}
              <button data-h-part="button" role="menuitem" class:selected={item.key === activeKey} aria-current={item.key === activeKey ? 'page' : undefined} disabled={!item.enabled || busy === item.key} onclick={() => execute(item)}>
                <Icon name="puzzle" size={20} /><span>{item.title}</span>{#if item.badge}<small>{item.badge}</small>{/if}{#if item.key === activeKey}<Icon name="check" size={16} />{/if}
              </button>
            {/each}
          </section>
        {/each}
      </div>
    {/if}
    {#if error}<p role="status">{error}</p>{/if}
  </div>
{/if}
<style>
  .plugin-contributions { display: flex; flex-wrap: wrap; gap: 8px; margin: 8px 0; }
  .plugin-contributions.navigation { display: flex; flex-direction: column; }
  .sidebar-contributions { flex-shrink: 0; }
  .sidebar-contributions > button > .nav-label { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .sidebar-contributions > button > span.nav-label { flex: 1; }
  .more-count { margin-left: auto; flex-shrink: 0; font-size: 12px; line-height: 20px; min-width: 20px; text-align: center; background: var(--h-selection); }
  .sidebar-overflow-menu { position: fixed; inset: auto; margin: 0; padding: 8px; overflow-y: auto; overscroll-behavior: contain; border: 1px solid var(--h-border); border-radius: var(--h-card-radius,12px); background: var(--h-surface-popup); color: var(--h-text-primary); box-shadow: 0 16px 40px color-mix(in srgb,var(--h-text-primary) 18%,transparent), 0 2px 8px color-mix(in srgb,var(--h-text-primary) 10%,transparent); }
  .more-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 8px 12px 12px; border-bottom: 1px solid var(--h-border); }
  .more-heading strong { font-size: calc(var(--h-font-size,16px) * .9375); font-weight: 600; }
  .more-heading > span, .more-group h3 { color: var(--h-text-secondary); font-size: calc(var(--h-font-size,16px) * .75); font-weight: 500; }
  .more-group { margin-top: 12px; }
  .more-group h3 { margin: 0 12px 4px; line-height: 1.5; overflow-wrap: anywhere; }
  .sidebar-overflow-menu .more-group button { width: 100%; min-height: 40px; padding: 8px 12px; gap: 12px; justify-content: flex-start; border: 0; box-shadow: none; border-radius: var(--h-radius,8px); font-size: calc(var(--h-font-size,16px) * .9375); color: var(--h-text-primary); text-align: left; }
  .more-group button > span { flex: 1; min-width: 0; overflow-wrap: anywhere; }
  .more-group button > :global(svg) { flex-shrink: 0; }
  .sidebar-overflow-menu .more-group button:hover:enabled { background: var(--h-control-hover); }
  .sidebar-overflow-menu .more-group button:active:enabled { background: var(--h-control-active); }
  .sidebar-overflow-menu .more-group button.selected { background: var(--h-selection); }
  .more-group button:focus-visible { outline-color: var(--h-focus-ring); outline-offset: -2px; }
  button { display: flex; align-items: center; gap: 8px; padding: 8px 12px; border: 1px solid var(--stroke); border-radius: 8px; background: transparent; color: inherit; font: inherit; cursor: pointer; }
  button:disabled { opacity: .45; cursor: default; }
  button:focus-visible { outline: 2px solid currentColor; outline-offset: 2px; }
  small { border-radius: 12px; padding: 2px 6px; background: var(--h-surface-card); }
  .plugin-view, p { width: 100%; }
</style>
