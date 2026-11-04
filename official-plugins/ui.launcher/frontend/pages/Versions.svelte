<script lang="ts">
  import Icon from '../../../ui.shell/frontend/components/Icon.svelte';
  import { action, errorMessage, taskActive, type Snapshot, type GameVersion } from '../../../ui.shell/frontend/lib/api';
  import { entrance } from '../../../ui.shell/frontend/lib/motion';
  // Keep selection primary within each row, with neutral 44px settings/launch actions (§2.8, §3.2).
  let { snapshot, mode = $bindable('instances'), onselect, onsettings, onlaunch, ondownloads, onchange }: {
    snapshot: Snapshot | null; mode: 'instances' | 'catalog' | 'configure';
    onselect: (id: string) => void; onsettings: (id: string) => void; onlaunch: (id: string) => Promise<void>; ondownloads: () => void; onchange: () => Promise<void>;
  } = $props();
  let selectedVersion = $state('');
  let newName = $state('');
  let expanded = $state('');
  let launching = $state('');
  let versions = $state<GameVersion[]>([]); let catalogKey = $state(''); let busy = $state(false); let error = $state('');
  let loader = $state(''); let fabricApi = $state(''); let optionsBusy = $state(false);
  let options = $state<{ loaders: { id: string; stable: boolean }[]; api: { id: string; version_number: string; version_type: string }[] }>({ loaders: [], api: [] });
  const installed = $derived(snapshot?.instances.filter(instance => instance.isInstalled) ?? []);
  const login = $derived(snapshot?.tasks.some(task => task.kind === 'login' && taskActive(task)));
  $effect(() => { const key = `${snapshot?.dataDirectory}/${snapshot?.settings.downloadSource}`; if (mode === 'catalog' && snapshot && catalogKey !== key) { catalogKey = key; void refreshVersions(); } });
  async function refreshVersions() { busy = true; error = ''; try { versions = await action<GameVersion[]>('versions'); } catch (e) { error = errorMessage(e); } finally { busy = false; } }
  async function configure(version: string) { selectedVersion = version; newName = version; loader = ''; fabricApi = ''; expanded = ''; error = ''; mode = 'configure'; optionsBusy = true; options = { loaders: [], api: [] }; try { const result = await action<typeof options>('fabricOptions', { version }); if (selectedVersion === version) options = result; } catch (e) { error = errorMessage(e); } finally { optionsBusy = false; } }
  async function install() { busy = true; error = ''; try { await action('createInstance', { version: selectedVersion, name: newName, components: loader ? { loader, fabricApi: fabricApi || null } : null }); await onchange(); mode = 'instances'; ondownloads(); } catch (e) { error = errorMessage(e); } finally { busy = false; } }
  async function launch(id: string) {
    if (launching || login || snapshot?.account.status !== 'signedIn' || snapshot.tasks.some(task => task.instanceId === id && taskActive(task))) return;
    launching = id; error = '';
    try { await onlaunch(id); } catch (e) { error = errorMessage(e); } finally { launching = ''; }
  }
</script>

<div class="versions-page">
  {#key mode}<div class="version-pane" in:entrance|global>
  {#if mode === 'instances'}
    <div class="section-row"><h2 class="section-label">已安装实例</h2></div>
    <div class="scroll-pane instances-list">
      {#each installed as instance (instance.id)}
        {@const activeTask = snapshot?.tasks.find(task => task.instanceId === instance.id && taskActive(task))}
        {@const launchTitle = activeTask ? activeTask.kind === 'launch' && activeTask.phase === 'running' ? '游戏正在运行' : '此实例有任务正在进行' : login ? '请等待账号登录完成' : snapshot?.account.status !== 'signedIn' ? '请先登录账号' : launching ? '正在提交启动请求' : `启动 ${instance.name}`}
        <div class="instance-row">
          <button class="instance-card" disabled={!instance.isInstalled} aria-pressed={snapshot?.selectedInstanceId === instance.id} onclick={() => onselect(instance.id)}>
            <span class="instance-text"><span class="instance-name">{instance.name}</span><span class="instance-version">{instance.gameVersion} · {instance.loaderName || (instance.components ? 'Fabric' : '原版')} · {instance.isInstalled ? '已安装' : '安装未完成'}</span></span>
            {#if snapshot?.selectedInstanceId === instance.id}<span class="current-instance">当前实例</span>{/if}
          </button>
          <button class="icon-button instance-settings" aria-label={`${instance.name}的实例设置`} onclick={() => onsettings(instance.id)}><Icon name="settings" size={22} /></button>
          <button class="icon-button instance-launch" aria-label={`启动实例：${instance.name}`} title={launchTitle} disabled={!!launching || !!login || snapshot?.account.status !== 'signedIn' || !!activeTask} onclick={() => launch(instance.id)}><Icon name="play" size={22} /></button>
        </div>
      {:else}<div class="empty-state">还没有安装完成的实例</div>{/each}
    </div>
    <div class="versions-footer"><p class="caption">{installed.length} 个隔离实例 · 各自保留模组与存档</p><button class="full-width" onclick={() => mode = 'catalog'}>添加版本</button></div>
  {:else if mode === 'catalog'}
    <div class="section-row"><h2 class="section-label">选择游戏版本</h2><button class="text-button" disabled={busy} onclick={refreshVersions}>刷新目录</button></div>
    <div class="scroll-pane catalog-list">
      {#each versions as version}
        <button class="version-card" in:entrance|global onclick={() => configure(version.id)}><span>{version.id}</span><span class="version-configure">配置实例 ›</span></button>
      {/each}
      {#if busy}<p class="caption" role="status">正在读取官方版本目录…</p>{/if}
    </div>
    <p class="caption versions-note">支持 1.18 起的官方正式版，使用{snapshot?.settings.downloadSource === 'bmclapi' ? ' BMCLAPI 镜像' : '官方源'}。</p>
  {:else}
    <div class="scroll-pane installation-form">
      <div class="install-name"><div><small>Minecraft</small><span>{selectedVersion}</span></div><input aria-label="新实例名称" maxlength="64" bind:value={newName} placeholder="实例名称" /></div>
      <h2 class="section-label">快捷安装</h2>
      {#each ['Fabric', 'Fabric API'] as component}
        <button class="component-choice" disabled={optionsBusy} aria-expanded={expanded === component} onclick={() => expanded = expanded === component ? '' : component}><span>{component}</span><span class="muted">{component === 'Fabric' ? loader || '不安装' : options.api.find(a => a.id === fabricApi)?.version_number ?? '不安装'}</span><span class:expanded={expanded === component}><Icon name="chevron" size={22} /></span></button>
        {#if expanded === component}<div class="component-options"><button class="full-width" onclick={() => { if (component === 'Fabric') { loader = ''; fabricApi = ''; } else fabricApi = ''; expanded = ''; }}>不安装</button>{#if component === 'Fabric'}{#each options.loaders as item}<button class="full-width" onclick={() => { loader = item.id; expanded = ''; }}>{item.id}{item.stable ? ' · 稳定版' : ''}</button>{/each}{:else}{#each options.api as item}<button class="full-width" disabled={!loader} onclick={() => { fabricApi = item.id; expanded = ''; }}>{item.version_number}{item.version_type === 'release' ? ' · 正式版' : ''}</button>{/each}{#if !loader}<p class="caption">请先选择 Fabric Loader。</p>{/if}{/if}</div>{/if}
      {/each}
      <p class="caption">默认安装原版。选择的 Fabric Loader / API 版本会固定保存。{optionsBusy ? '正在查询兼容组件…' : ''}</p>
    </div>
    <div class="versions-footer"><p class="caption">游戏文件按哈希校验；离开页面后安装会继续。</p><button class="full-width" disabled={busy} onclick={install}>{busy ? '正在创建…' : '开始安装'}</button></div>
  {/if}
  </div>{/key}
  {#if error}<p class="issue" role="alert">{error}</p>{/if}
</div>
