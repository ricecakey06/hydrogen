<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { PhysicalPosition, PhysicalSize } from '@tauri-apps/api/dpi';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { invoke, action, errorMessage, taskActive, type Snapshot, type Profile } from './lib/api';
  import { animateText, entrance, fadeSlide } from './lib/motion';
  import Icon from './components/Icon.svelte';
  import Versions from '../../ui.launcher/frontend/pages/Versions.svelte';
  import SettingsForm from '../../ui.launcher/frontend/pages/SettingsForm.svelte';
  import Plugins from '../../plugins.manager/frontend/Plugins.svelte';
  import Mods from '../../ui.launcher/frontend/pages/Mods.svelte';
  import Home from '../../ui.launcher/frontend/pages/Home.svelte';
  import Downloads from '../../ui.launcher/frontend/pages/Downloads.svelte';
  import CrashReport from '../../ui.launcher/frontend/pages/CrashReport.svelte';

  import Profiles from '../../ui.launcher/frontend/pages/Profiles.svelte';

  type Page = 'profiles' | 'home' | 'versions' | 'instance' | 'settings' | 'mods' | 'downloads' | 'crash';
  type ResizeDirection = Parameters<ReturnType<typeof getCurrentWindow>['startResizeDragging']>[0];
  let page = $state<Page>('home');
  let sidebarCollapsed = $state(false);
  let navigationDirection = $state(1);
  let settingsTab = $state('general');
  let versionMode = $state<'instances' | 'catalog' | 'configure'>('instances');
  let selectedPluginId = $state('');
  let snapshot = $state<Snapshot | null>(null);
  let editingInstanceId = $state('');
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
  function pageTitle(active: Page) { return active === 'instance' ? `版本设置 · ${editing?.name ?? ''}` : active === 'mods' ? `模组管理 · ${editing?.name ?? ''}` : ({ home: '', versions: '版本选择', profiles: '档案管理', settings: '启动器设置', downloads: '下载管理', crash: '崩溃分析' } as Partial<Record<Page, string>>)[active]; }
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
  function fit() { scale = Math.min(window.innerWidth / 1000, window.innerHeight / 714); }
  let requestSequence = 0, appliedSequence = 0;
  function stable<T>(value: T, previous: T | undefined): T { return previous !== undefined && JSON.stringify(value) === JSON.stringify(previous) ? previous : value; }
  async function load() { const sequence = ++requestSequence; const next = await invoke<Snapshot>('app_snapshot'); if (sequence < appliedSequence) return; appliedSequence = sequence; next.plugins = stable(next.plugins, snapshot?.plugins); next.instances = stable(next.instances, snapshot?.instances); next.settings = stable(next.settings, snapshot?.settings); next.javaRuntimes = stable(next.javaRuntimes, snapshot?.javaRuntimes); next.account = stable(next.account, snapshot?.account); snapshot = next; document.documentElement.dataset.material = next.appearance.material; }
  async function operate(type: string, fields: Record<string, unknown> = {}) { try { await action(type, fields); await load(); } catch (e) { error = errorMessage(e); } }
  let downloadError = $state('');
  async function startInstall(id: string) { downloadError = ''; navigate('downloads'); try { await action('repair', { id }); await load(); } catch (e) { downloadError = errorMessage(e); } }
  function navigate(next: Page) { navigationDirection = next === 'home' ? -1 : 1; page = next; error = ''; selectedPluginId = ''; void load().catch(e => error = errorMessage(e)); }
  function showSettings(id: string) { editingInstanceId = id; navigate('instance'); }
  function back() {
    if (selectedPluginId) { selectedPluginId = ''; return; }
    if (page === 'versions' && versionMode !== 'instances') { versionMode = versionMode === 'configure' ? 'catalog' : 'instances'; return; }
    if (page === 'mods') { navigate('instance'); return; }
    if (page === 'instance') { versionMode = 'instances'; navigate('versions'); return; }
    navigate('home');
  }
  async function select(id: string) {
    try { await invoke('instance_select', { id }); await load(); navigate('home'); }
    catch (e) { error = errorMessage(e); }
  }
  async function launchInstance(id: string) {
    await invoke('instance_select', { id });
    await action('launch', { id });
    await load();
    navigate('home');
  }
  async function openModal(title: string, description: string, install = false, confirm?: () => Promise<void>, confirmLabel = '继续') {
    restoreFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    modalError = ''; modal = { title, description, install, confirm, confirmLabel }; await tick(); modalElement?.focus();
  }
  async function closeModal() { if (modalPending) return; modal = null; await tick(); if (restoreFocus?.isConnected) restoreFocus.focus(); }
  async function confirmModal() { if (!modal?.confirm || modalPending) return; modalPending = true; modalError = ''; let complete = false; try { await modal.confirm(); complete = true; } catch (e) { modalError = errorMessage(e); } finally { modalPending = false; } if (complete) await closeModal(); }
  function deleteProfile(profile: Profile) {
    void openModal('删除档案？', `将从此启动器移除“${profile.name}”${profile.kind === 'microsoft' ? '及其本机登录凭据' : ''}。游戏文件和存档会保留。`, false, async () => {
      await action('profileDelete', { id: profile.id, confirmation: profile.name }); await load();
    }, '删除档案');
  }
  function login() { void operate('signIn'); }
  function deleteInstance() { if (!editing) return; const instance = editing; void openModal('删除此实例？', `将删除“${instance.name}”的版本目录及其中的文件。共享目录中的存档、模组和资源会保留。`, false, async () => { await action('deleteInstance', { id: instance.id, confirmation: instance.name }); await load(); versionMode = 'instances'; navigate('versions'); }, '确认删除'); }
  async function importLegacy() { try { const preview = await action<{ token: string; instances: number; bytes: number; settings: boolean; source: string } | null>('importLegacyPreview'); if (preview) void openModal('导入旧版数据', `来源：${preview.source}。发现 ${preview.instances} 个实例，约 ${(preview.bytes / 1024 / 1024).toFixed(1)} MiB。将复制实例、模组、存档和缓存到当前目录。${preview.settings ? '当前目录尚无设置，将同时导入旧版 Java、内存、窗口和下载设置。' : '当前启动器设置保持原样。'}旧目录不会被修改；账号凭据和旧 DLL 插件不导入。`, false, async () => { await action('importLegacyCommit', { token: preview.token }); await load(); settingsTab = 'general'; navigate('settings'); }, '开始导入'); } catch (e) { error = errorMessage(e); } }
  async function installPlugin() { try { const preview = await action<{token: string; id: string; name: string; version: string; permissions: string[]; dependencies: unknown[]; previousVersion: string | null} | null>('pluginPreview'); if (preview) void openModal(preview.previousVersion ? '升级本地插件' : '安装本地插件', `${preview.name}（${preview.id}）${preview.previousVersion ? `${preview.previousVersion} → ` : ''}${preview.version}。请求能力：${preview.permissions.join('、') || '无'}。依赖：${preview.dependencies.length ? JSON.stringify(preview.dependencies) : '无'}。${preview.permissions.includes('native.code') ? '此插件包含本机代码，将拥有与启动器相同的系统权限。' : ''}请确认信任此插件；安装后按需激活。`, false, async () => { await action('pluginInstall', { token: preview.token }); await load(); }, preview.previousVersion ? '确认升级' : '确认安装'); } catch (e) { error = errorMessage(e); } }
  function uninstallPlugin(plugin: Snapshot['plugins'][number]) { void openModal('卸载插件？', `将卸载“${plugin.name}”。依赖它的插件会进入不可用状态。`, false, async () => { await action('pluginUninstall', { id: plugin.id, digest: plugin.digest, confirmed: true }); await load(); }, '确认卸载'); }
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
    let stopped = false;
    let unlisten: UnlistenFn | undefined;
    void listen<{ token: string; count: number }>('mod-files-dropped', event => {
      if (page === 'mods' && editing) {
        const id = editing.id;
        void openModal('导入模组', `将 ${event.payload.count} 个文件导入“${editing.name}”。同名文件不会被覆盖。`, false, async () => { await action('modsDropped', { id, token: event.payload.token }); await load(); }, '导入');
      } else void openModal('导入模组', '请先打开目标实例的模组管理页面，再拖入模组文件。');
    }).then(value => { if (stopped) value(); else unlisten = value; }).catch(e => error = errorMessage(e));
    return () => { stopped = true; unlisten?.(); };
  });

</script>

<svelte:window onresize={fit} onkeydown={keydown} onpointermove={resizeMove} onpointerup={resizeEnd} onpointercancel={resizeEnd} />
<div class="viewport">
  <div class="design-surface" class:sidebar-collapsed={sidebarCollapsed} style={`--ui-scale:${scale}`}>
    <header class="titlebar" data-tauri-drag-region inert={!!modal}>
      <span class="sidebar-mark"><button class="icon-button sidebar-toggle" aria-label={sidebarCollapsed ? '展开侧栏' : '收起侧栏'} title={sidebarCollapsed ? '展开侧栏' : '收起侧栏'} aria-expanded={!sidebarCollapsed} aria-controls="main-navigation" onclick={() => sidebarCollapsed = !sidebarCollapsed}><Icon name="panel" size={22} width={1.6} /></button></span><span class="brand" data-tauri-drag-region>Hydrogen</span>
      <button class="icon-button caption-button" aria-label="最小化" onclick={() => caption('minimize')}><Icon name="minus" size={24} width={2.5} /></button>
      <button class="icon-button caption-button close-button" aria-label="关闭" onclick={() => caption('close')}><Icon name="close" size={21} width={2.5} /></button>
    </header>
    <main class="window-body" inert={!!modal}>
      <nav id="main-navigation" class="sidebar" aria-label="主导航">
        <button class:active={page === 'home'} aria-label="启动游戏" title={sidebarCollapsed ? '启动游戏' : undefined} onclick={() => navigate('home')}><Icon name="circle-play" /><span class="nav-label">启动游戏</span></button>
        <button class:active={page === 'versions' || page === 'instance' || page === 'mods'} aria-label="版本选择" title={sidebarCollapsed ? '版本选择' : undefined} onclick={() => { versionMode = 'instances'; navigate('versions'); }}><Icon name="cube" /><span class="nav-label">版本选择</span></button>
        <button class:active={page === 'profiles'} aria-label="档案管理" title={sidebarCollapsed ? '档案管理' : undefined} onclick={() => navigate('profiles')}><Icon name="profiles" /><span class="nav-label">档案管理</span></button>
        <p class="nav-section">扩展</p>
        <button class:active={page === 'settings' && settingsTab === 'plugins'} aria-label="插件" title={sidebarCollapsed ? '插件' : undefined} onclick={() => { settingsTab = 'plugins'; navigate('settings'); }}><Icon name="puzzle" /><span class="nav-label">插件</span></button>
        <div class="sidebar-footer"><button class:active={page === 'settings' && settingsTab === 'general'} aria-label="启动器设置" title={sidebarCollapsed ? '启动器设置' : undefined} onclick={() => { settingsTab = 'general'; navigate('settings'); }}><Icon name="gear" /><span class="nav-label">启动器设置</span></button></div>
      </nav>
      {#each [page] as activePage (activePage)}
      <div class="route-layer" aria-hidden={page !== activePage} inert={page !== activePage} in:fadeSlide|global={{ direction: navigationDirection }} out:fadeSlide={{ direction: navigationDirection }}>
      {#if activePage === 'home'}
        <Home {snapshot} onlogin={login} oncancel={id => { void operate('cancelTask', { id }); }} onlaunch={() => selected && void operate('launch', { id: selected.id })} onversions={() => { versionMode = 'instances'; navigate('versions'); }} oninstance={() => selected && showSettings(selected.id)} onsettings={() => navigate('profiles')} ondownloads={() => navigate('downloads')} onlogs={id => { void operate('openLogs', { id }); }} oncrash={showCrash} />
      {:else}
        <div class="page-shell"><div class="page-heading"><button class="icon-button back-button" aria-label="返回" onclick={back}><Icon name="back" size={22} width={2.5} /></button><h1 use:animateText={pageTitle(activePage)}>{pageTitle(activePage)}</h1></div>
          {#if activePage === 'crash'}{#key crashTaskId}<CrashReport task={crashTask} onhome={() => navigate('home')} />{/key}
          {:else if activePage === 'profiles'}<Profiles {snapshot} onchange={load} ondelete={deleteProfile} />
          {:else if activePage === 'versions'}<Versions {snapshot} bind:mode={versionMode} onselect={select} onsettings={showSettings} onlaunch={launchInstance} ondownloads={() => { downloadError = ''; navigate('downloads'); }} onchange={load} />
          {:else if activePage === 'instance'}<SettingsForm onprofiles={() => navigate('profiles')} instance={editing} {snapshot} onmods={() => navigate('mods')} onlogin={login} onchange={load} ondelete={deleteInstance} onimport={importLegacy} onrepair={id => { void startInstall(id); }} />
          {:else if activePage === 'mods'}<Mods instance={editing} {snapshot} onchange={load} onconfirm={(title, description, confirm) => { void openModal(title, description, false, confirm, '确认删除'); }} />
          {:else if activePage === 'settings'}
            <div class="settings-tabs"><div class="segmented" role="group" aria-label="启动器设置分类"><button class:selected={settingsTab === 'general'} aria-pressed={settingsTab === 'general'} onclick={() => { settingsTab = 'general'; selectedPluginId = ''; }}>常规</button><button class:selected={settingsTab === 'plugins'} aria-pressed={settingsTab === 'plugins'} onclick={() => settingsTab = 'plugins'}>插件</button></div></div>
            {#key settingsTab}<div class="tab-content" in:entrance|global>{#if settingsTab === 'general'}<SettingsForm onprofiles={() => navigate('profiles')} {snapshot} onmods={() => navigate('mods')} onlogin={login} onchange={load} ondelete={deleteInstance} onimport={importLegacy} onrepair={id => { void startInstall(id); }} />{:else}<Plugins plugins={snapshot?.plugins ?? []} bind:selectedId={selectedPluginId} oninstall={installPlugin} onuninstall={uninstallPlugin} onrefresh={refreshPlugins} />{/if}</div>{/key}
          {:else if activePage === 'downloads'}<Downloads {snapshot} onchange={load} onhome={() => navigate('home')} onrepair={startInstall} startError={downloadError} />{/if}
        </div>
      {/if}
      </div>
      {/each}
      {#if error}<p class="error-toast" role="alert" in:entrance use:animateText={error}>{error}</p>{/if}
    </main>
    {#if modal}
      <div class="modal-overlay">
        <button class="modal-scrim" tabindex="-1" disabled={modalPending} aria-label="关闭对话框" onclick={() => closeModal()}></button>
        <dialog open class="modal" aria-modal="true" aria-labelledby="modal-title" tabindex="-1" bind:this={modalElement} in:entrance>
          <h2 id="modal-title">{modal.title}</h2><p>{modal.description}</p>
          {#if modalError}<p class="issue" role="alert">{modalError}</p>{/if}
          <div class="modal-actions"><button disabled={modalPending} onclick={() => closeModal()}>{modal.install || modal.confirm ? '取消' : '知道了'}</button>{#if modal.confirm}<button class="primary" disabled={modalPending} onclick={confirmModal}>{modalPending ? '正在处理…' : modal.confirmLabel}</button>{:else if modal.install}<button class="primary" disabled>安装插件</button>{/if}</div>
        </dialog>
      </div>
    {/if}
  </div>
  {#if !modal}{#each grips as grip}<div class={`resize-grip ${grip.edge}`} role="presentation" onpointerdown={event => resize(event, grip.direction)}></div>{/each}{/if}
</div>
