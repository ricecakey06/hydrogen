<script lang="ts">
  import { t } from '../../../ui.shell/frontend/lib/i18n';
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
  let loader = $state(''); let loaderKind = $state('fabric'); let fabricApi = $state(''); let optionsBusy = $state(false);
  let filter = $state('release'); let search = $state(''); let optionsError = $state('');
  type Loader = { id: string; stable: boolean };
  let options = $state<{ loaders: Loader[]; api: { id: string; version_number: string; version_type: string }[] }>({ loaders: [], api: [] });
  let forgeOptions = $state<Record<string, Loader[]>>({ forge: [], neoforge: [] });
  let loaded = $state<string[]>([]); let request = 0;
  const visibleVersions = $derived(versions.filter(v => (filter === 'all' || v.versionType === filter) && v.id.toLowerCase().includes(search.toLowerCase())));
  const installed = $derived(snapshot?.instances.filter(instance => instance.isInstalled) ?? []);
  const login = $derived(snapshot?.tasks.some(task => task.kind === 'login' && taskActive(task)));
  $effect(() => { const key = `${snapshot?.dataDirectory}/${snapshot?.settings.downloadSource}`; if (mode === 'catalog' && snapshot && catalogKey !== key) { catalogKey = key; void refreshVersions(); } });
  async function refreshVersions() { busy = true; error = ''; try { versions = await action<GameVersion[]>('versions'); } catch (e) { error = errorMessage(e); } finally { busy = false; } }
  function configure(version: string) {
    request++; selectedVersion = version; newName = version; loader = ''; loaderKind = 'fabric'; fabricApi = ''; expanded = ''; error = ''; optionsError = ''; mode = 'configure'; optionsBusy = false;
    options = { loaders: [], api: [] }; forgeOptions = { forge: [], neoforge: [] }; loaded = [];
  }
  async function expandComponent(component: string) {
    if (expanded === component) { expanded = ''; return; }
    expanded = component; optionsError = '';
    const kind = component === 'NeoForge' ? 'neoforge' : component === 'Forge' ? 'forge' : 'fabric';
    if (loaded.includes(kind)) return;
    const current = ++request;
    optionsBusy = true;
    try {
      if (kind === 'fabric') {
        const result = await action<typeof options>('fabricOptions', { version: selectedVersion });
        if (current === request) options = result;
      } else {
        const result = await action<Loader[]>('forgeOptions', { version: selectedVersion, kind });
        if (current === request) forgeOptions[kind] = result;
      }
      if (current === request) loaded = [...loaded, kind];
    } catch (e) { if (current === request) optionsError = errorMessage(e); }
    finally { if (current === request) optionsBusy = false; }
  }
  function chooseLoader(kind: string, version: string) {
    loaderKind = kind; loader = version; fabricApi = ''; expanded = '';
  }
  async function install() { busy = true; error = ''; try { await action('createInstance', { version: selectedVersion, name: newName, components: loader ? { kind: loaderKind, loader, fabricApi: fabricApi || null } : null }); await onchange(); mode = 'instances'; ondownloads(); } catch (e) { error = errorMessage(e); } finally { busy = false; } }
  async function launch(id: string) {
    if (!snapshot || launching || login || snapshot.tasks.some(task => task.instanceId === id && taskActive(task))) return;
    launching = id; error = '';
    try { await onlaunch(id); } catch (e) { error = errorMessage(e); } finally { launching = ''; }
  }
</script>

<div class="versions-page">
  {#key mode}<div class="version-pane" in:entrance|global>
  {#if mode === 'instances'}
    <div class="section-row"><h2 class="section-label">{t("已安装实例")}</h2></div>
    <div class="scroll-pane instances-list">
      {#each installed as instance (instance.id)}
        {@const activeTask = snapshot?.tasks.find(task => task.instanceId === instance.id && taskActive(task))}
        {@const launchTitle = activeTask ? activeTask.kind === 'launch' && activeTask.phase === 'running' ? t("游戏正在运行") : t("此实例有任务正在进行") : login ? t("请等待账号登录完成") : launching ? t("正在提交启动请求") : snapshot?.account.status !== 'signedIn' ? t("前往档案管理登录") : t("启动 {0}", instance.name)}
        <div class="instance-row">
          <button class="instance-card" disabled={!instance.isInstalled} aria-pressed={snapshot?.selectedInstanceId === instance.id} onclick={() => onselect(instance.id)}>
            <span class="instance-text"><span class="instance-name">{instance.name}</span><span class="instance-version">{instance.gameVersion} · {instance.loaderName || (instance.components ? 'Fabric' : t("原版"))} · {instance.isInstalled ? t("已安装") : t("安装未完成")}</span></span>
            {#if snapshot?.selectedInstanceId === instance.id}<span class="current-instance">{t("当前实例")}</span>{/if}
          </button>
          <button class="icon-button instance-settings" aria-label={t("{0}的实例设置", instance.name)} onclick={() => onsettings(instance.id)}><Icon name="settings" size={22} /></button>
          <button class="icon-button instance-launch" aria-label={t("启动实例：{0}", instance.name)} title={launchTitle} disabled={!!launching || !!login || !!activeTask} onclick={() => launch(instance.id)}><Icon name="play" size={22} /></button>
        </div>
      {:else}<div class="empty-state">{t("还没有安装完成的实例")}</div>{/each}
    </div>
    <div class="versions-footer"><p class="caption">{installed.length} {t("个隔离实例 · 各自保留模组与存档")}</p><button class="full-width" onclick={() => mode = 'catalog'}>{t("添加版本")}</button></div>
  {:else if mode === 'catalog'}
    <div class="section-row"><h2 class="section-label">{t("选择游戏版本")}</h2><button class="text-button" disabled={busy} onclick={refreshVersions}>{t("刷新目录")}</button></div>
    <div class="field-row">
      <input aria-label={t("搜索游戏版本")} placeholder={t("搜索版本，例如 1.12.2 或 24w14a")} bind:value={search} />
      <select aria-label={t("版本类型")} bind:value={filter}><option value="release">{t("正式版")}</option><option value="snapshot">{t("快照 / 预发布版")}</option><option value="all">{t("全部版本")}</option></select>
    </div>
    <div class="scroll-pane catalog-list">
      {#each visibleVersions as version}
        <button class="version-card" in:entrance|global onclick={() => configure(version.id)}><span>{version.id}{version.versionType === 'snapshot' ? t(" · 快照") : ''}</span><span class="version-configure">{t("配置实例 ›")}</span></button>
      {/each}
      {#if busy}<p class="caption" role="status">{t("正在读取官方版本目录…")}</p>{:else if !visibleVersions.length}<p class="caption">{t("没有匹配的版本。")}</p>{/if}
    </div>
    <p class="caption versions-note">{t("支持全部官方正式版（含 1.18 以下）与快照版，使用")}{snapshot?.settings.downloadSource === 'bmclapi' ? t(" BMCLAPI 镜像") : t("官方源")}。</p>
  {:else}
    <div class="scroll-pane installation-form">
      <div class="install-name"><div><small>Minecraft</small><span>{selectedVersion}</span></div><input aria-label={t("新实例名称")} maxlength="64" bind:value={newName} placeholder={t("实例名称")} /></div>
      <h2 class="section-label">{t("快捷安装")}</h2>
      {#each ['Fabric', 'Forge', 'NeoForge', 'Fabric API'] as component}
        {@const kind = component === 'NeoForge' ? 'neoforge' : component === 'Forge' ? 'forge' : 'fabric'}
        {@const choices = kind === 'fabric' ? options.loaders : forgeOptions[kind]}
        <button class="component-choice" disabled={optionsBusy || busy} aria-expanded={expanded === component} onclick={() => expandComponent(component)}>
          <span>{component}</span><span class="muted">{component === 'Fabric API' ? options.api.find(a => a.id === fabricApi)?.version_number ?? t("不安装") : loaderKind === kind && loader ? loader : t("不安装")}</span><span class:expanded={expanded === component}><Icon name="chevron" size={22} /></span>
        </button>
        {#if expanded === component}
          <div class="component-options">
            <button class="full-width" disabled={busy} onclick={() => { if (component === 'Fabric API') fabricApi = ''; else if (loaderKind === kind) { loader = ''; fabricApi = ''; } expanded = ''; }}>{t("不安装")}</button>
            {#if optionsBusy}<p class="caption" role="status">{t("正在查询兼容组件…")}</p>
            {:else if optionsError}<p class="issue" role="alert">{optionsError}</p><button class="text-button" onclick={() => { expanded = ''; void expandComponent(component); }}>{t("重试")}</button>
            {:else if component === 'Fabric API'}
              {#each options.api as item}<button class="full-width" disabled={busy || !loader || loaderKind !== 'fabric'} onclick={() => { fabricApi = item.id; expanded = ''; }}>{item.version_number}{item.version_type === 'release' ? t(" · 正式版") : ''}</button>{/each}
              {#if !loader || loaderKind !== 'fabric'}<p class="caption">{t("请先选择 Fabric Loader。")}</p>{:else if !options.api.length}<p class="caption">{t("此版本暂无兼容的 Fabric API。")}</p>{/if}
            {:else}
              {#each choices as item}<button class="full-width" disabled={busy} onclick={() => chooseLoader(kind, item.id)}>{item.id}{item.stable ? t(" · 稳定版") : ''}</button>{/each}
              {#if !choices.length}<p class="caption">{t("此 Minecraft 版本暂无兼容的")} {component}。</p>{/if}
            {/if}
          </div>
        {/if}
      {/each}
      <p class="caption">{t("默认安装原版。Fabric、Forge、NeoForge 只能选择一种；Fabric API 仅适用于 Fabric。")}</p>
    </div>
    <div class="versions-footer"><p class="caption">{t("游戏文件按哈希校验；离开页面后安装会继续。")}</p><button class="full-width" disabled={busy || !newName.trim()} onclick={install}>{busy ? t("正在创建…") : t("开始安装")}</button></div>
  {/if}
  </div>{/key}
  {#if error}<p class="issue" role="alert">{error}</p>{/if}
</div>
