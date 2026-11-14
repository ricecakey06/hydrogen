<script lang="ts">
  import { t, i18n, loadLanguage } from './lib/i18n';
  import { onMount, tick } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { PhysicalPosition, PhysicalSize } from '@tauri-apps/api/dpi';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { invoke, action, errorMessage, taskActive, type Snapshot, type Profile, type InstanceInfo } from './lib/api';
  import { animateText, entrance, fadeSlide } from './lib/motion';
  import PluginSlot from './components/PluginSlot.svelte';
  import ThemeRecovery from './components/ThemeRecovery.svelte';
  import AppearanceSettings from './components/AppearanceSettings.svelte';
  import { attachTheme, syncThemeSettings, cancelTheme, refreshThemeRoot } from './lib/theme';
  let themeRoot = $state<HTMLDivElement>();
  $effect(() => { refreshThemeRoot(themeRoot); });
  import PluginPanel from './components/PluginPanel.svelte';
  import { uiState, type Contribution } from './lib/contributions';
  import Icon from './components/Icon.svelte';
  import Versions from '../../ui.launcher/frontend/pages/Versions.svelte';
  import SettingsForm, { type InstanceSettingsTab } from '../../ui.launcher/frontend/pages/SettingsForm.svelte';
  import About from '../../ui.launcher/frontend/pages/About.svelte';
  import Plugins from '../../plugins.manager/frontend/Plugins.svelte';
  import Mods from '../../ui.launcher/frontend/pages/Mods.svelte';
  import Home from '../../ui.launcher/frontend/pages/Home.svelte';
  import Downloads from '../../ui.launcher/frontend/pages/Downloads.svelte';
  import Resources, { type ResourceDetail } from '../../ui.launcher/frontend/pages/Resources.svelte';
  import CrashReport from '../../ui.launcher/frontend/pages/CrashReport.svelte';

  import Modpacks from '../../ui.launcher/frontend/pages/Modpacks.svelte';
  import Profiles from '../../ui.launcher/frontend/pages/Profiles.svelte';

  type Page = 'profiles' | 'home' | 'versions' | 'instance' | 'settings' | 'downloads' | 'crash' | 'resources' | 'plugin';
  type ResizeDirection = Parameters<ReturnType<typeof getCurrentWindow>['startResizeDragging']>[0];
  let page = $state<Page>('home');
  let externalView = $state<Contribution | null>(null);
  function openContribution(item: Contribution) { externalView = item; navigate('plugin'); }
  let sidebarCollapsed = $state(false);
  let navigationDirection = $state(1);
  type SettingsTab = 'general' | 'theme' | 'plugins' | 'about';
  let settingsTab = $state<SettingsTab>('general');
  function selectSettingsTab(next: SettingsTab, pluginId = '') {
    if (settingsTab === 'theme' && next !== 'theme') void cancelTheme();
    settingsTab = next;
    selectedPluginId = pluginId;
  }
  let versionMode = $state<'instances' | 'catalog' | 'configure'>('instances');
  let selectedPluginId = $state('');
  let snapshot = $state<Snapshot | null>(null);
  let resourceTrail = $state<ResourceDetail[]>([]);
  const resourcesAvailable = $derived(snapshot?.plugins.some(plugin => plugin.id === 'minecraft.resources' && plugin.enabled && !plugin.problem));
  const modpacksAvailable = $derived(snapshot?.plugins.some(plugin => plugin.id === 'minecraft.modpacks' && plugin.enabled && !plugin.problem));
  let resourceExpanded = $state(false);
  const showResourceMenu = $derived(page === 'resources' && resourceExpanded);
  let editingInstanceId = $state('');
  let instanceTab = $state<InstanceSettingsTab>('general');
  let crashTaskId = $state('');
  const shownCrashes = new Set<string>();
  const crashTask = $derived(snapshot?.tasks.find(task => task.id === crashTaskId));
  let error = $state('');
  let scale = $state(1);
  let modal = $state<{ title: string; description: string; install?: boolean; confirm?: () => Promise<void>; confirmLabel?: string } | null>(null);
  let modalPending = $state(false); let modalError = $state('');
  let modalElement = $state<HTMLElement>();
  let restoreFocus: HTMLElement | null = null;
  type ResizeDrag = { pointer: number; target: HTMLElement; direction: ResizeDirection; startX: number; startY: number; x: number; y: number; origin?: { width: number; height: number; x: number; y: number; dpi: number } };
  let resizeDrag: ResizeDrag | null = null;
  let resizeQueued = false;
  const selected = $derived(snapshot?.instances.find(i => i.id === snapshot?.selectedInstanceId));
  const editing = $derived(snapshot?.instances.find(i => i.id === editingInstanceId) ?? selected);
  function pageTitle(active: Page) { return { plugin: externalView?.title ?? t("插件"), home: '', versions: t("版本选择"), instance: t("版本设置"), profiles: t("档案管理"), settings: t("启动器设置"), downloads: t("下载管理"), crash: t("崩溃分析"), resources: t("资源下载") }[active]; }
  function showCrash(id: string) { crashTaskId = id; navigate('crash'); }
  $effect(() => {
    if (modal) return;
    const crashed = snapshot?.tasks.filter(task => task.kind === 'launch' && task.phase === 'failed' && task.exitCode != null && task.exitCode !== 0 && (task.crashReport || task.crashError) && !shownCrashes.has(task.id)) ?? [];
    if (crashed.length) { for (const task of crashed) shownCrashes.add(task.id); showCrash(crashed[0].id); }
  });
  const grips: { direction: ResizeDirection; edge: string }[] = [
    { direction: 'North', edge: 'top' }, { direction: 'South', edge: 'bottom' }, { direction: 'West', edge: 'left' }, { direction: 'East', edge: 'right' },
    { direction: 'NorthWest', edge: 'top-left' }, { direction: 'NorthEast', edge: 'top-right' }, { direction: 'SouthWest', edge: 'bottom-left' }, { direction: 'SouthEast', edge: 'bottom-right' },
  ];
  function fit() { scale = 1; }
  let requestSequence = 0, appliedSequence = 0;
  function stable<T>(value: T, previous: T | undefined): T { return previous !== undefined && JSON.stringify(value) === JSON.stringify(previous) ? previous : value; }
  async function load() { const sequence = ++requestSequence; const next = await invoke<Snapshot>('app_snapshot'); if (sequence < appliedSequence) return; appliedSequence = sequence; next.plugins = stable(next.plugins, snapshot?.plugins); next.instances = stable(next.instances, snapshot?.instances); next.settings = stable(next.settings, snapshot?.settings); next.javaRuntimes = stable(next.javaRuntimes, snapshot?.javaRuntimes); next.account = stable(next.account, snapshot?.account); snapshot = next; syncThemeSettings(next.settings, next.dataDirectory, next.plugins); document.documentElement.dataset.material = next.appearance.material; await loadLanguage(next.dataDirectory, next.settings.locale, next.plugins.find(p => p.id === 'i18n')); }
  async function operate(type: string, fields: Record<string, unknown> = {}) { try { await action(type, fields); await load(); } catch (e) { error = errorMessage(e); } }
  let downloadError = $state('');
  function showDownloads() { downloadError = ''; navigate('downloads'); }
  async function importModpack() {
    try { if (await action<string | null>('modpackImport')) showDownloads(); }
    catch (e) { error = errorMessage(e); }
  }
  async function startInstall(id: string, type: 'repair' | 'retryInstallation' = 'repair') { downloadError = ''; if (page !== 'downloads') navigate('downloads'); try { await action(type, { id }); await load(); } catch (e) { downloadError = errorMessage(e); } }
  function navigate(next: Page) { if (page === 'settings' && next !== 'settings') void cancelTheme(); navigationDirection = next === 'home' ? -1 : 1; page = next; error = ''; selectedPluginId = ''; resourceTrail = []; void load().catch(e => error = errorMessage(e)); }
  function toggleResources() {
    if (page === 'resources') { resourceExpanded = !resourceExpanded; return; }
    resourceExpanded = true;
    navigate('resources');
  }
  function showSettings(id: string) { editingInstanceId = id; instanceTab = 'general'; navigate('instance'); }
  function back() {
    if (selectedPluginId) { selectedPluginId = ''; return; }
    if (page === 'resources' && resourceTrail.length) { resourceTrail = resourceTrail.slice(0, -1); return; }
    if (page === 'versions' && versionMode !== 'instances') { versionMode = versionMode === 'configure' ? 'catalog' : 'instances'; return; }
    if (page === 'instance') { versionMode = 'instances'; navigate('versions'); return; }
    navigate('home');
  }
  async function select(id: string) {
    try { await invoke('instance_select', { id }); await load(); navigate('home'); }
    catch (e) { error = errorMessage(e); }
  }
  async function launchInstance(id: string) {
    await invoke('instance_select', { id });
    if (snapshot?.account.status !== 'signedIn') {
      navigate('profiles');
      return;
    }
    await action('launch', { id });
    await load();
    navigate('home');
  }
  async function openModal(title: string, description: string, install = false, confirm?: () => Promise<void>, confirmLabel = t("继续")) {
    restoreFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    modalError = ''; modal = { title, description, install, confirm, confirmLabel }; await tick(); modalElement?.focus();
  }
  async function closeModal() { if (modalPending) return; modal = null; await tick(); if (restoreFocus?.isConnected) restoreFocus.focus(); }
  async function confirmModal() { if (!modal?.confirm || modalPending) return; modalPending = true; modalError = ''; let complete = false; try { await modal.confirm(); complete = true; } catch (e) { modalError = errorMessage(e); } finally { modalPending = false; } if (complete) await closeModal(); }
  function deleteProfile(profile: Profile) {
    void openModal(t("删除档案？"), t("将从此启动器移除“{0}”{1}。游戏文件和存档会保留。", profile.name, profile.kind === 'microsoft' ? t("及其本机登录凭据") : ''), false, async () => {
      await action('profileDelete', { id: profile.id, confirmation: profile.name }); await load();
    }, t("删除档案"));
  }
  function login() { void operate('signIn'); }
  function deleteInstance() { if (!editing) return; const instance = editing; void openModal(t("删除此实例？"), t("将删除“{0}”的版本目录及其中的文件。共享目录中的存档、模组和资源会保留。", instance.name), false, async () => { await action('deleteInstance', { id: instance.id, confirmation: instance.name }); await load(); versionMode = 'instances'; navigate('versions'); }, t("确认删除")); }
  async function importLegacy() { try { const preview = await action<{ token: string; instances: number; bytes: number; settings: boolean; source: string } | null>('importLegacyPreview'); if (preview) void openModal(t("导入旧版数据"), t("来源：{0}。发现 {1} 个实例，约 {2} MiB。将复制实例、模组、存档和缓存到当前目录。{3}旧目录不会被修改；账号凭据和旧 DLL 插件不导入。", preview.source, preview.instances, (preview.bytes / 1024 / 1024).toFixed(1), preview.settings ? t("当前目录尚无设置，将同时导入旧版 Java、内存、窗口和下载设置。") : t("当前启动器设置保持原样。")), false, async () => { await action('importLegacyCommit', { token: preview.token }); await load(); settingsTab = 'general'; navigate('settings'); }, t("开始导入")); } catch (e) { error = errorMessage(e); } }
  async function installPlugin() { try { const preview = await action<{token: string; id: string; name: string; version: string; permissions: string[]; dependencies: unknown[]; previousVersion: string | null} | null>('pluginPreview'); if (preview) void openModal(preview.previousVersion ? t("升级本地插件") : t("安装本地插件"), t("{0}（{1}）{2}{3}。请求能力：{4}。依赖：{5}。{6}请确认信任此插件；安装后按需激活。", preview.name, preview.id, preview.previousVersion ? `${preview.previousVersion} → ` : '', preview.version, preview.permissions.join('、') || t("无"), preview.dependencies.length ? JSON.stringify(preview.dependencies) : t("无"), preview.permissions.includes('native.code') ? t("此插件包含本机代码，将拥有与启动器相同的系统权限。") : ''), false, async () => { await action('pluginInstall', { token: preview.token }); await load(); }, preview.previousVersion ? t("确认升级") : t("确认安装")); } catch (e) { error = errorMessage(e); } }
  function uninstallPlugin(plugin: Snapshot['plugins'][number]) { void openModal(t("卸载插件？"), t("将卸载“{0}”。依赖它的插件会进入不可用状态。", plugin.name), false, async () => { await action('pluginUninstall', { id: plugin.id, digest: plugin.digest, confirmed: true }); await load(); }, t("确认卸载")); }
  async function refreshPlugins() { try { await action('pluginRefresh'); await load(); } catch (e) { error = errorMessage(e); } }
  function keydown(event: KeyboardEvent) {
    if (event.key === 'Escape') { event.preventDefault(); if (modal) void closeModal(); else back(); }
    if (event.key === 'Tab' && modal && modalElement) {
      const controls = [...modalElement.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), [tabindex="0"]')];
      const first = controls[0], last = controls.at(-1);
      if (!first) { event.preventDefault(); return; }
      if (event.shiftKey && (document.activeElement === first || document.activeElement === modalElement)) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && (document.activeElement === last || document.activeElement === modalElement)) { event.preventDefault(); first.focus(); }
    }
  }
  async function caption(action: 'minimize' | 'close') { try { await getCurrentWindow()[action](); } catch (e) { error = errorMessage(e); } }
  async function resize(event: PointerEvent, direction: ResizeDirection) {
    if (event.button !== 0) return;
    event.preventDefault();
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    const drag: ResizeDrag = { pointer: event.pointerId, target, direction, startX: event.screenX, startY: event.screenY, x: event.screenX, y: event.screenY };
    resizeDrag = drag;
    try {
      const native = getCurrentWindow();
      const [size, position, dpi] = await Promise.all([native.innerSize(), native.outerPosition(), native.scaleFactor()]);
      drag.origin = { width: size.width, height: size.height, x: position.x, y: position.y, dpi };
      queueResize();
    } catch (e) { error = errorMessage(e); resizeDrag = null; }
  }
  function resizeMove(event: PointerEvent) {
    if (!resizeDrag || resizeDrag.pointer !== event.pointerId) return;
    resizeDrag.x = event.screenX; resizeDrag.y = event.screenY; queueResize();
  }
  function queueResize() {
    if (resizeQueued || !resizeDrag?.origin) return;
    resizeQueued = true;
    requestAnimationFrame(async () => {
      const drag = resizeDrag;
      if (!drag?.origin) { resizeQueued = false; return; }
      const origin = drag.origin;
      const x = drag.x, y = drag.y;
      const dx = (x - drag.startX) * origin.dpi, dy = (y - drag.startY) * origin.dpi;
      const horizontal = /East|West/.test(drag.direction), vertical = /North|South/.test(drag.direction);
      const byWidth = (origin.width + (drag.direction.includes('West') ? -dx : dx)) / 1000;
      const byHeight = (origin.height + (drag.direction.includes('North') ? -dy : dy)) / 714;
      const scale = Math.max(.3 * origin.dpi, horizontal && (!vertical || Math.abs(dx / 1000) >= Math.abs(dy / 714)) ? byWidth : byHeight);
      const width = Math.round(1000 * scale), height = Math.round(714 * scale);
      try {
        const native = getCurrentWindow();
        await native.setSize(new PhysicalSize(width, height));
        if (/West|North/.test(drag.direction)) await native.setPosition(new PhysicalPosition(origin.x + (drag.direction.includes('West') ? origin.width - width : 0), origin.y + (drag.direction.includes('North') ? origin.height - height : 0)));
      } catch (e) { error = errorMessage(e); }
      finally { resizeQueued = false; if (resizeDrag === drag && (x !== drag.x || y !== drag.y)) queueResize(); }
    });
  }
  function resizeEnd(event: PointerEvent) {
    if (resizeDrag?.pointer !== event.pointerId) return;
    resizeDrag.target.releasePointerCapture(event.pointerId); resizeDrag = null;
  }
  const pollDelay = $derived(snapshot?.tasks.some(task => taskActive(task) && task.phase !== 'running') ? 100 : 1800);
  let ready = false;
  $effect(() => {
    const delay = pollDelay;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        await load();
        if (!ready) { ready = true; requestAnimationFrame(() => { void invoke('ui_ready').catch(() => {}); }); }
      } catch (e) { error = errorMessage(e); }
      finally { if (!stopped) timer = setTimeout(poll, delay); }
    }
    void poll();
    return () => { stopped = true; clearTimeout(timer); };
  });
  onMount(() => {
    fit();
    const detachTheme = attachTheme(themeRoot!, snapshot?.settings.theme ?? { id: 'builtin:default', mode: 'system', material: 'auto' });
    let stopped = false;
    const unlisteners: UnlistenFn[] = [];
    void listen<{ pluginId: string; digest: string; generation: number; id: string; state: { title?: string; badge?: string; enabled?: boolean; visible?: boolean } }>('plugin-ui-update', event => {
      const v = event.payload;
      const key=v.pluginId + '/' + v.digest + '/' + v.id;
      uiState.update(state => (state[key]?.generation ?? 0) > v.generation ? state : { ...state, [key]: { ...v.state, generation: v.generation } });
    }).then(value => { if (stopped) value(); else unlisteners.push(value); });
    void listen<{ pluginId: string; digest: string; generation: number }>('plugin-ui-stop', event => {
      const v=event.payload, prefix=v.pluginId + '/' + v.digest + '/';
      uiState.update(state => Object.fromEntries(Object.entries(state).filter(([key,value]) => !key.startsWith(prefix) || (value.generation ?? 0)>v.generation)));
    }).then(value => { if (stopped) value(); else unlisteners.push(value); });
    void listen<{ pluginId: string; id: string }>('plugin-navigate', event => {
      const p = snapshot?.plugins.find(p => p.id === event.payload.pluginId && p.enabled && !p.problem);
      const view = p?.views.find(v => v.id === event.payload.id);
      if (p && view) openContribution({ ...view, pluginId: p.id, digest: p.digest, key: p.id + '/' + p.digest + '/' + view.id, enabled: true });
    }).then(value => { if (stopped) value(); else unlisteners.push(value); });
    void listen<{ token: string; count: number }>('mod-files-dropped', event => {
      if (page === 'instance' && instanceTab === 'mods' && editing) {
        const id = editing.id;
        void openModal(t("导入模组"), t("将 {0} 个文件导入“{1}”。同名文件不会被覆盖。", event.payload.count, editing.name), false, async () => { await action('modsDropped', { id, token: event.payload.token }); await load(); }, t("导入"));
      } else void openModal(t("导入模组"), t("请先打开目标实例的模组管理页面，再拖入模组文件。"));
    }).then(value => { if (stopped) value(); else unlisteners.push(value); }).catch(e => error = errorMessage(e));
    void listen<{ message: string } | null>('modpack-drop', event => {
      if (event.payload) error = errorMessage(event.payload);
      else showDownloads();
    }).then(value => { if (stopped) value(); else unlisteners.push(value); }).catch(e => error = errorMessage(e));
    return () => { stopped = true; detachTheme(); unlisteners.forEach(unlisten => unlisten()); };
  });

</script>

<svelte:window onresize={fit} onkeydown={keydown} onpointermove={resizeMove} onpointerup={resizeEnd} onpointercancel={resizeEnd} />
{#key $i18n}
<div class="viewport">
  <div bind:this={themeRoot} data-h-theme-root data-h-page={page} data-h-sidebar={sidebarCollapsed ? "collapsed" : "expanded"} class="design-surface" class:sidebar-collapsed={sidebarCollapsed} style={`--ui-scale:${scale}`}>
    <div data-h-theme-slot="window.background" aria-hidden="true"></div>
    <header data-h-part="titlebar" class="titlebar" data-tauri-drag-region inert={!!modal}>
      <span class="sidebar-mark"><button data-h-part="button" class="icon-button sidebar-toggle" aria-label={sidebarCollapsed ? t("展开侧栏") : t("收起侧栏")} title={sidebarCollapsed ? t("展开侧栏") : t("收起侧栏")} aria-expanded={!sidebarCollapsed} aria-controls="main-navigation" onclick={() => sidebarCollapsed = !sidebarCollapsed}><Icon name="panel" size={22} width={1.6} /></button></span><span class="brand-space" data-tauri-drag-region><span class="brand" data-h-part="brand" aria-label="Hydrogen" data-tauri-drag-region><span data-h-builtin-brand>Hydrogen</span><img data-h-part="brand-image" hidden alt="" /></span></span>
      <div data-h-theme-slot="titlebar.decoration" aria-hidden="true"></div>
      <PluginSlot {snapshot} location="titlebar.actions" context={{ instanceId: selected?.id }} onchange={load} />
      <button data-h-part="button" class="icon-button caption-button" aria-label={t("最小化")} onclick={() => caption('minimize')}><Icon name="minus" size={24} width={2.5} /></button>
      <button data-h-part="button" class="icon-button caption-button close-button" aria-label={t("关闭")} onclick={() => caption('close')}><Icon name="close" size={21} width={2.5} /></button>
    </header>
    <main class="window-body" inert={!!modal}>
      <nav data-h-part="sidebar" id="main-navigation" class="sidebar" aria-label={t("主导航")}>
        <div data-h-theme-slot="sidebar.background" aria-hidden="true"></div><div data-h-theme-slot="sidebar.decoration" aria-hidden="true"></div>
        <button data-h-part="nav-item" class:active={page === 'home'} aria-label={t("启动游戏")} title={sidebarCollapsed ? t("启动游戏") : undefined} onclick={() => navigate('home')}><Icon name="circle-play" /><span class="nav-label">{t("启动游戏")}</span></button>
        <button data-h-part="nav-item" class:active={page === 'versions' || page === 'instance'} aria-label={t("版本选择")} title={sidebarCollapsed ? t("版本选择") : undefined} onclick={() => { versionMode = 'instances'; navigate('versions'); }}><Icon name="cube" /><span class="nav-label">{t("版本选择")}</span></button>
        {#if resourcesAvailable}
          <div class="resource-menu">
            <button data-h-part="nav-item" class="resource-trigger" class:active={page === 'resources' && !showResourceMenu} aria-label={t("资源下载")} title={sidebarCollapsed ? t("资源下载") : undefined} aria-expanded={showResourceMenu} aria-controls={showResourceMenu ? 'resource-navigation' : undefined} onclick={toggleResources}>
              <Icon name="download" /><span class="nav-label">{t("资源下载")}</span>
              <span class="nav-chevron" class:expanded={showResourceMenu}><Icon name="chevron" size={16} /></span>
            </button>
            {#if showResourceMenu}
              <div id="resource-navigation" class="resource-subnav" role="group" aria-label={t("资源分类")} in:entrance>
                <button data-h-part="nav-item" class="active" aria-current="page" aria-label={t("模组下载")} title={sidebarCollapsed ? t("模组下载") : undefined} onclick={() => resourceTrail = []}>
                  <Icon name="puzzle" size={18} /><span class="nav-label">{t("模组下载")}</span>
                </button>
              </div>
            {/if}
          </div>
        {/if}
        <button data-h-part="nav-item" class:active={page === 'profiles'} aria-label={t("档案管理")} title={sidebarCollapsed ? t("档案管理") : undefined} onclick={() => navigate('profiles')}><Icon name="profiles" /><span class="nav-label">{t("档案管理")}</span></button>
        <PluginSlot {snapshot} location="sidebar.main" context={{ instanceId: selected?.id }} activeKey={page === 'plugin' ? externalView?.key : ''} onnavigate={openContribution} onchange={load} />
        <p class="nav-section">{t("扩展")}</p>
        <button data-h-part="nav-item" class:active={page === 'settings' && settingsTab === 'plugins'} aria-label={t("插件")} title={sidebarCollapsed ? t("插件") : undefined} onclick={() => { selectSettingsTab('plugins'); navigate('settings'); }}><Icon name="puzzle" /><span class="nav-label">{t("插件")}</span></button>
        <div class="sidebar-footer"><PluginSlot {snapshot} location="sidebar.footer" context={{ instanceId: selected?.id }} activeKey={page === 'plugin' ? externalView?.key : ''} onnavigate={openContribution} onchange={load} /><button data-h-part="nav-item" class:active={page === 'settings' && settingsTab !== 'plugins'} aria-label={t("启动器设置")} title={sidebarCollapsed ? t("启动器设置") : undefined} onclick={() => { selectSettingsTab('general'); navigate('settings'); }}><Icon name="gear" /><span class="nav-label">{t("启动器设置")}</span></button></div>
      </nav>
      <div class="theme-content" data-h-part="content"><div data-h-theme-slot="content.background" aria-hidden="true"></div><div data-h-theme-slot="content.decoration" aria-hidden="true"></div>
      {#each [page] as activePage (activePage)}
      <div data-h-part="page" class="route-layer" aria-hidden={page !== activePage} inert={page !== activePage} in:fadeSlide|global={{ direction: navigationDirection }} out:fadeSlide={{ direction: navigationDirection }}>
      {#if activePage === 'plugin'}
        {#if externalView && snapshot?.plugins.some(p => p.id === externalView?.pluginId && p.digest === externalView?.digest && p.enabled && !p.problem)}
          <div class="page-shell plugin-page"><h1>{externalView.title}</h1>{#key externalView.key}<PluginPanel pluginId={externalView.pluginId} panelId={externalView.panel!} context={{ instanceId: selected?.id }} locale={snapshot?.settings.locale} height={520} onerror={message => error = message} />{/key}</div>
        {:else}<p class="empty-state">{t("插件不可用，请重新打开。")}</p>{/if}
      {:else if activePage === 'home'}
        <Home {snapshot} onlogin={login} onmicrosoft={() => { void operate('signInMicrosoft'); }} oncancel={id => { void operate('cancelTask', { id }); }} onlaunch={() => selected && void operate('launch', { id: selected.id })} onversions={() => { versionMode = 'instances'; navigate('versions'); }} oninstance={() => selected && showSettings(selected.id)} onsettings={() => navigate('profiles')} ondownloads={() => navigate('downloads')} onlogs={id => { void operate('openLogs', { id }); }} oncrash={showCrash} />
      {:else}
        <div class="page-shell"><div data-h-part="page-heading" class="page-heading"><button data-h-part="button" class="icon-button back-button" aria-label={t("返回")} onclick={back}><Icon name="back" size={22} width={2.5} /></button><h1 use:animateText={pageTitle(activePage)}>{pageTitle(activePage)}</h1></div>
          {#if activePage === 'crash'}{#key crashTaskId}<CrashReport task={crashTask} onhome={() => navigate('home')} />{/key}
          {:else if activePage === 'profiles'}<Profiles {snapshot} onchange={load} ondelete={deleteProfile} />
          {:else if activePage === 'versions'}<Versions {snapshot} bind:mode={versionMode} onselect={select} onsettings={showSettings} onlaunch={launchInstance} onrepair={startInstall} onimportpack={modpacksAvailable ? importModpack : undefined} ondownloads={showDownloads} onchange={load} />
          {:else if activePage === 'instance'}
            <PluginSlot {snapshot} location="instance.toolbar" context={{ instanceId: editing?.id }} onchange={load} />
            {#snippet packPanel(instance: InstanceInfo)}<Modpacks {instance} {snapshot} onchange={load} />{/snippet}
            <SettingsForm onprofiles={() => navigate('profiles')} instance={editing} {snapshot} bind:instanceTab exportPack={modpacksAvailable ? packPanel : undefined} onlogin={login} onchange={load} ondelete={deleteInstance} onimport={importLegacy} onrepair={id => { void startInstall(id); }}>
              {#snippet mods(instance: InstanceInfo)}<Mods {instance} {snapshot} onchange={load} onconfirm={(title, description, confirm) => { void openModal(title, description, false, confirm, t("确认删除")); }} />{/snippet}
            </SettingsForm>
          {:else if activePage === 'settings'}
            <div class="settings-tabs"><div class="segmented" role="group" aria-label={t("启动器设置分类")}>
              <button data-h-part="button" class:selected={settingsTab === 'general'} aria-pressed={settingsTab === 'general'} onclick={() => selectSettingsTab('general')}>{t("常规")}</button>
              <button data-h-part="button" class:selected={settingsTab === 'theme'} aria-pressed={settingsTab === 'theme'} onclick={() => selectSettingsTab('theme')}>{t("主题")}</button>
              <button data-h-part="button" class:selected={settingsTab === 'plugins'} aria-pressed={settingsTab === 'plugins'} onclick={() => selectSettingsTab('plugins')}>{t("插件")}</button>
              <button data-h-part="button" class:selected={settingsTab === 'about'} aria-pressed={settingsTab === 'about'} onclick={() => selectSettingsTab('about')}>{t("关于")}</button>
            </div></div>
            <!-- Keep drafts and open settings panels while visiting the theme tab. -->
            <div class="tab-content" hidden={settingsTab !== 'general'}>
              <SettingsForm onprofiles={() => navigate('profiles')} {snapshot} onlogin={login} onchange={load} ondelete={deleteInstance} onimport={importLegacy} onrepair={id => { void startInstall(id); }} />
            </div>
            {#if settingsTab !== 'general'}{#key settingsTab}<div class="tab-content" in:entrance|global>
              {#if settingsTab === 'theme'}<div class="form-page"><div class="scroll-pane form-scroll"><AppearanceSettings {snapshot} onplugins={id => selectSettingsTab('plugins', id)} /></div></div>
              {:else if settingsTab === 'plugins'}<Plugins plugins={snapshot?.plugins ?? []} bind:selectedId={selectedPluginId} oninstall={installPlugin} onuninstall={uninstallPlugin} onrefresh={refreshPlugins} />
              {:else}<About version={snapshot?.version} />{/if}
            </div>{/key}{/if}
          {:else if activePage === 'downloads'}<Downloads {snapshot} onchange={load} onhome={() => navigate('home')} onretry={id => startInstall(id, 'retryInstallation')} startError={downloadError} />
          {:else if activePage === 'resources'}
            {#if resourcesAvailable}<Resources {snapshot} bind:trail={resourceTrail} onchange={load} />
            {:else}<p class="empty-state">{t("资源下载插件不可用。")}</p>{/if}
          {/if}
        </div>
      {/if}
      </div>
      {/each}
      <PluginSlot {snapshot} location="status.items" context={{ instanceId: selected?.id }} onchange={load} />
      {#if error}<p class="error-toast" role="alert" in:entrance use:animateText={error}>{error}</p>{/if}
      </div>
    </main>
    {#if modal}
      <div class="modal-overlay">
        <button class="modal-scrim" tabindex="-1" disabled={modalPending} aria-label={t("关闭对话框")} onclick={() => closeModal()}></button>
        <dialog data-h-scope="popup" open class="modal" aria-modal="true" aria-labelledby="modal-title" tabindex="-1" bind:this={modalElement} in:entrance>
          <h2 id="modal-title">{modal.title}</h2><p>{modal.description}</p>
          {#if modalError}<p class="issue" role="alert">{modalError}</p>{/if}
          <div class="modal-actions"><button data-h-part="button" disabled={modalPending} onclick={() => closeModal()}>{modal.install || modal.confirm ? t("取消") : t("知道了")}</button>{#if modal.confirm}<button data-h-part="button" data-h-variant="primary" class="primary" disabled={modalPending} onclick={confirmModal}>{modalPending ? t("正在处理…") : modal.confirmLabel}</button>{:else if modal.install}<button data-h-part="button" data-h-variant="primary" class="primary" disabled>{t("安装插件")}</button>{/if}</div>
        </dialog>
      </div>
    {/if}
  </div>
  {#if !modal}{#each grips as grip}<div class={`resize-grip ${grip.edge}`} role="presentation" onpointerdown={event => resize(event, grip.direction)}></div>{/each}{/if}
</div>
{/key}
<ThemeRecovery />
<style>
  .plugin-page { padding: 24px; overflow: auto; }
  .plugin-page h1 { margin: 0 0 20px; font-size: calc(var(--h-font-size,16px) * 1.375); font-weight: 600; }
</style>
