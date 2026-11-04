<script lang="ts">
  import Icon from '../../ui.shell/frontend/components/Icon.svelte';
  import PluginPanel from '../../ui.shell/frontend/components/PluginPanel.svelte';
  import { invoke, action, errorMessage, type PluginInfo, type RunReport } from '../../ui.shell/frontend/lib/api';
  import { animateText, entrance, fadeSlide } from '../../ui.shell/frontend/lib/motion';
  let { plugins, selectedId = $bindable(''), oninstall, onrefresh, onuninstall }: { plugins: PluginInfo[]; selectedId: string; oninstall: () => void; onrefresh: () => void; onuninstall: (plugin: PluginInfo) => void } = $props();
  let filter = $state('all');
  let listDirection = $state(1);
  let query = $state('');
  let busy = $state(false);
  let error = $state('');
  let report = $state<RunReport | null>(null);
  let panel = $state<{ pluginId: string; panelId: string } | null>(null);
  const builtIns = $derived(plugins.filter(p => p.origin === 'builtIn'));
  const external = $derived(plugins.filter(p => p.origin !== 'builtIn'));
  const filtered = $derived(plugins.filter(p => (filter === 'all' || (filter === 'builtin') === (p.origin === 'builtIn')) && `${p.name} ${p.description} ${p.id}`.toLowerCase().includes(query.toLowerCase())));
  const selected = $derived(plugins.find(p => p.id === selectedId));
  const list = $derived({ filter, query, items: filtered, externalCount: external.length });
  function changeFilter(next: string) {
    listDirection = ['all', 'builtin', 'external'].indexOf(next) < ['all', 'builtin', 'external'].indexOf(filter) ? -1 : 1;
    filter = next;
  }
  async function toggle(plugin: PluginInfo) { busy = true; error = ''; panel = null; try { await action('pluginToggle', { id: plugin.id, enabled: !plugin.enabled }); onrefresh(); } catch (e) { error = errorMessage(e); } finally { busy = false; } }
  async function directory() { try { await action('pluginDirectory'); } catch (e) { error = errorMessage(e); } }
  function details(id: string) { selectedId = id; panel = null; report = null; error = ''; }
  async function run(pluginId: string, command: string) {
    busy = true; panel = null; error = ''; report = null;
    try { report = await invoke<RunReport>('plugin_run', { pluginId, command }); }
    catch (e) { error = errorMessage(e); }
    finally { busy = false; }
  }
  async function reload(pluginId: string) {
    busy = true; panel = null; error = ''; report = null;
    try { report = await invoke<RunReport>('plugin_reload', { pluginId }); onrefresh(); }
    catch (e) { error = errorMessage(e); }
    finally { busy = false; }
  }
</script>

<div class="plugins-page">
  <div class="plugins-toolbar"><div><h2>安装、启用和管理插件</h2><p class="plugin-label">{builtIns.length} 个内置模块 · {external.length} 个外置插件</p></div><button class="icon-button" aria-label="刷新插件列表" onclick={onrefresh}><Icon name="refresh" size={18} /></button><button class="primary" onclick={oninstall}>＋ 安装插件</button></div>
  {#if error && !selected}<p class="issue" role="alert">{error}</p>{/if}
  {#if !selected}
    <div class="plugin-filters"><div class="segmented" role="group" aria-label="插件分类">
      <button class:selected={filter === 'all'} aria-pressed={filter === 'all'} onclick={() => changeFilter('all')}>全部（{plugins.length}）</button>
      <button class:selected={filter === 'builtin'} aria-pressed={filter === 'builtin'} onclick={() => changeFilter('builtin')}>内置（{builtIns.length}）</button>
      <button class:selected={filter === 'external'} aria-pressed={filter === 'external'} onclick={() => changeFilter('external')}>外置（{external.length}）</button>
    </div><input class="plugin-search" aria-label="搜索插件" placeholder="搜索名称、描述或插件 ID" bind:value={query} /></div>
    <div class="plugin-list-stage">
    {#each [list] as visible (visible)}
    <div class="scroll-pane plugin-list" in:fadeSlide|global={{ direction: listDirection, compact: true }} out:fadeSlide={{ direction: listDirection, compact: true }}>
      {#each [{ key: 'builtIn', title: '内置插件' }, { key: 'installed', title: '已安装插件' }, { key: 'development', title: '开发插件' }] as group}
        {@const items = visible.items.filter(p => p.origin === group.key)}
        {#if items.length}
          <h3 class="plugin-group">{group.title}</h3>
          {#each items as plugin (plugin.id)}
            <div class="plugin-row"><button class="plugin-row-main" onclick={() => details(plugin.id)}>
              <span class="plugin-icon"><Icon name={plugin.id === 'ui.shell' ? 'settings' : plugin.id === 'ui.launcher' ? 'cube' : 'puzzle'} size={23} width={1.7} /></span>
              <span class="plugin-row-text"><span class="plugin-name">{plugin.name}</span><span class="plugin-description">{plugin.description}</span><span class="plugin-id">{plugin.id}</span></span>
            </button><div class="plugin-row-state">{#if plugin.required}<span>必需</span>{:else}<button class="plugin-toggle" role="switch" aria-checked={plugin.enabled} aria-label={`${plugin.enabled ? '停用' : '启用'} ${plugin.name}`} disabled={busy} onclick={() => toggle(plugin)}>{plugin.enabled ? '已启用' : '已停用'}</button>{/if}<span class:running={plugin.enabled && !plugin.problem} class="plugin-status"><i></i>{plugin.enabled ? (plugin.problem ? '依赖不可用' : (plugin.required ? '运行中' : '按需激活')) : '已停用'}</span></div></div>
          {/each}
        {/if}
      {/each}
      {#if !visible.items.length}<div class="plugin-empty"><span class="empty-plugin-icon"><Icon name="puzzle" size={28} width={1.5} /></span><h3>{visible.query ? '没有匹配的插件' : '尚未加载外置插件'}</h3><p class="plugin-label">{visible.query ? '请尝试其他名称、描述或插件 ID。' : '选择本地 .hplugin 包安装插件。'}</p><button onclick={oninstall}>安装本地插件</button></div>{/if}
      {#if visible.filter === 'all' && !visible.externalCount}<h3 class="plugin-group">外置插件</h3><p class="plugin-label empty-installed">尚未加载外置插件。可从本地目录添加扩展。</p>{/if}
    </div>
    {/each}
    </div>
  {:else}
    <div class="scroll-pane plugin-details" in:entrance>
      <button class="text-button detail-back" onclick={() => details('')}>‹ 返回插件列表</button>
      <section class="plugin-detail-card"><div class="plugin-detail-heading"><span class="plugin-icon"><Icon name={selected.required ? 'settings' : 'puzzle'} size={28} width={1.7} /></span><div><h2>{selected.name}</h2><p class="plugin-id">{selected.id}</p></div><div class="plugin-detail-state"><span>{selected.required ? '必需' : selected.enabled ? '已启用' : '已停用'}</span><small>{selected.enabled ? selected.problem ? '依赖不可用' : '按需激活' : '已停用'}</small></div></div>
        <p>{selected.description}</p><div class="plugin-meta"><div><span>来源</span><strong>{selected.origin === 'builtIn' ? '内置模块' : selected.origin === 'installed' ? '本地插件包' : '开发目录'}</strong></div><div><span>版本</span><strong>{selected.version}</strong></div><div><span>作者</span><strong>{selected.origin === 'builtIn' ? 'Hydrogen' : '未声明'}</strong></div></div>
        <p class="plugin-state-note">{selected.required ? '此模块是当前部署所需的依赖。' : selected.runtime === 'native' ? '此插件包含本机代码，拥有与启动器相同的系统权限。' : '在独立 JavaScript 运行时中按需激活。'}</p>
      </section>
      {#if selected.problem}<p class="issue">{selected.problem}</p>{/if}
      {#if !selected.required}<div class="inline-actions"><button disabled={busy} onclick={() => toggle(selected)}>{selected.enabled ? '停用插件' : '启用插件'}</button><button disabled={busy || !selected.enabled} onclick={() => reload(selected.id)}>校验并重载</button>{#if selected.origin !== 'development'}<button disabled={busy} onclick={() => onuninstall(selected)}>卸载插件</button>{/if}</div>{/if}
      <details class="expander"><summary>依赖与服务</summary><p class="plugin-label">{selected.dependencies.length ? JSON.stringify(selected.dependencies) : '无外部依赖'}</p><p class="plugin-label">{selected.required ? '由宿主提供，不声明外部脚本权限。' : '声明的能力'}</p><p class="permission-list">{selected.permissions.join(' · ') || '无外部能力'}</p></details>
      {#if selected.commands.length}<h3 class="plugin-group">插件命令</h3><div class="inline-actions plugin-commands">{#each selected.commands as command}<button disabled={busy || !selected.enabled || !!selected.problem} onclick={() => run(selected.id, command.id)}>{command.title}</button>{/each}</div>{/if}
      {#if selected.panels.length}<h3 class="plugin-group">自定义面板</h3><div class="inline-actions">{#each selected.panels as p}<button disabled={busy || !selected.enabled || !!selected.problem} onclick={() => panel = { pluginId: selected.id, panelId: p.id }}>{p.title}</button>{/each}</div>{/if}
      {#if busy}<p class="operation-status" role="status"><span class="spinner"><Icon name="spinner" size={18} /></span>正在执行插件命令…</p>{/if}
      {#if report}<p class="notice" role="status" in:entrance use:animateText={report.notifications.join(' · ')}>{report.notifications.join(' · ') || '命令已完成'}</p><details class="expander"><summary>运行诊断</summary><p class="caption">代次 {report.generation} · JS 堆 {(report.heapBytes / 1024).toFixed(0)} KiB · 注册 {report.registrations} 项</p></details>{/if}
      {#if error}<p class="issue" role="alert" in:entrance use:animateText={error}>{error}</p>{/if}
      {#if panel}{#key `${panel.pluginId}/${panel.panelId}`}<div class="plugin-panel"><PluginPanel pluginId={panel.pluginId} panelId={panel.panelId} onerror={message => error = message} /></div>{/key}{/if}
    </div>
  {/if}
  <footer class="plugins-footer"><span class="plugin-label">预装与外置插件统一管理 · 插件包经校验后安装</span><button class="text-button" onclick={directory}>插件目录</button></footer>
</div>
