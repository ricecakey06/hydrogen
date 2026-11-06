<script module lang="ts">
  export type InstanceSettingsTab = string;
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';
  import { t, i18n } from '../../../ui.shell/frontend/lib/i18n';
  import { action, errorMessage, instanceLoaderLabel, taskActive, type InstanceInfo, type Snapshot, type Settings, type JavaRuntime } from '../../../ui.shell/frontend/lib/api';
  import Icon from '../../../ui.shell/frontend/components/Icon.svelte';
  import InstanceIcon from '../components/InstanceIcon.svelte';
  import PluginPanel from '../../../ui.shell/frontend/components/PluginPanel.svelte';
  import PluginSlot from '../../../ui.shell/frontend/components/PluginSlot.svelte';
  import { contributions, uiState } from '../../../ui.shell/frontend/lib/contributions';
  import { entrance, animateText } from '../../../ui.shell/frontend/lib/motion';
  let { instance, snapshot, instanceTab = $bindable('general'), mods, exportPack, onlogin, onprofiles, onchange, ondelete, onimport, onrepair }: {
    instance?: InstanceInfo; snapshot: Snapshot | null; instanceTab?: InstanceSettingsTab;
    mods?: Snippet<[InstanceInfo]>; exportPack?: Snippet<[InstanceInfo]>;
    onlogin: () => void; onprofiles: () => void; onchange: () => Promise<void>; ondelete: () => void;
    onimport: () => void; onrepair: (id: string) => void;
  } = $props();
  const formId = $props.id();
  const canExport = $derived(!!exportPack && !!instance?.isInstalled && !snapshot?.readOnly);
  const pluginTabs = $derived(contributions(snapshot, 'instance.tabs', { instanceId: instance?.id }, $uiState).filter(item => item.panel));
  const instanceTabs = $derived(([
    { id: 'general', label: t("常规"), icon: 'settings' },
    { id: 'mods', label: t("模组管理"), icon: 'puzzle' },
    { id: 'packExport', label: t("导出整合包"), icon: 'cube' },
  ] as const).filter(tab => tab.id !== 'packExport' || canExport).map(tab => ({ ...tab, id: String(tab.id), enabled: true })).concat(pluginTabs.map(item => ({ id: item.key, label: item.title, icon: 'puzzle', enabled: item.enabled }))));
  let openedTools = $state({ mods: false, packExport: false });
  let locale = $state('zh-CN');
  const legacyImport = $derived(snapshot?.tasks.find(task => task.kind === 'legacyImport'));
  let instanceName = $state('');
  let source = $state('official');
  let useModMirror = $state(true);
  let java = $state('');
  let memory = $state(4096);
  let inheritMemory = $state(true);
  let inheritJvm = $state(true);
  let isolation = $state('auto');
  let windowMode = $state('default');
  let windowWidth = $state(854);
  let windowHeight = $state(480);
  let afterLaunch = $state('minimize');
  let jvm = $state('');
  let loaded = $state('');
  let error = $state(''); let message = $state(''); let busy = $state(false);
  $effect(() => {
    const key = `${snapshot?.dataDirectory}/${instance?.id ?? 'global'}`;
    if (!snapshot || loaded === key) return;
    loaded = key;
    instanceTab = 'general';
    openedTools = { mods: false, packExport: false };
    const settings = snapshot.settings;
    locale = $i18n.languages.some(language => language.locale === settings.locale) ? settings.locale : $i18n.locale;
    instanceName = instance?.name ?? '';
    java = instance ? instance.overrides.java ?? '' : settings.java ?? '';
    memory = instance?.overrides.maxMemoryMib ?? settings.maxMemoryMib; inheritMemory = instance?.overrides.maxMemoryMib == null;
    jvm = instance?.overrides.jvmArguments ?? settings.jvmArguments; inheritJvm = instance?.overrides.jvmArguments == null;
    isolation = settings.isolateGameDirectory == null ? 'auto' : settings.isolateGameDirectory ? 'isolated' : 'shared';
    source = settings.downloadSource; windowMode = settings.windowMode; windowWidth = settings.windowWidth; windowHeight = settings.windowHeight; afterLaunch = settings.afterLaunch;
    useModMirror = settings.useModMirror ?? true;
    error = ''; message = '';
  });
  $effect(() => {
    if (!canExport && instanceTab === 'packExport') instanceTab = 'general';
    if (!instanceTabs.some(tab => tab.id === instanceTab && tab.enabled)) instanceTab = 'general';
  });
  function selectInstanceTab(tab: InstanceSettingsTab) {
    instanceTab = tab;
    if (tab === 'mods' || tab === 'packExport') openedTools[tab] = true;
  }
  function onTabKeydown(event: KeyboardEvent, index: number) {
    const last = instanceTabs.length - 1;
    let next: number;
    switch (event.key) {
      case 'ArrowLeft': next = index === 0 ? last : index - 1; break;
      case 'ArrowRight': next = index === last ? 0 : index + 1; break;
      case 'Home': next = 0; break;
      case 'End': next = last; break;
      default: return;
    }
    event.preventDefault();
    while (!instanceTabs[next].enabled && next !== index) next = (next + 1) % instanceTabs.length;
    selectInstanceTab(instanceTabs[next].id);
    document.getElementById(`${formId}-tab-${instanceTab}`)?.focus();
  }
  async function operate(type: string, fields: Record<string,unknown> = {}) { busy = true; error = ''; message = ''; try { const result = await action<unknown>(type, fields); await onchange(); return result; } catch (e) { error = errorMessage(e); } finally { busy = false; } }
  async function chooseJava() { const runtime = await operate('javaChoose') as JavaRuntime | null | undefined; if (runtime) java = runtime.path; }
  function repair() { if (instance && !busy) onrepair(instance.id); }
  async function save() {
    if (!snapshot) return; busy = true; error = ''; message = '';
    try {
      if (instance) { await action('saveOverrides', { id: instance.id, overrides: { java: java || null, maxMemoryMib: inheritMemory ? null : Number(memory), jvmArguments: inheritJvm ? null : jvm } }); }
      else { const settings: Settings = { ...snapshot.settings, locale: languageAvailable ? locale : snapshot.settings.locale, isolateGameDirectory: isolation === 'auto' ? null : isolation === 'isolated', java: java || null, maxMemoryMib: Number(memory), jvmArguments: jvm, downloadSource: source, useModMirror, windowMode, windowWidth: Number(windowWidth), windowHeight: Number(windowHeight), afterLaunch }; await action('saveSettings', { settings }); }
      await onchange(); message = t("设置已保存");
    } catch (e) { error = errorMessage(e); } finally { busy = false; }
  }
  const languageAvailable = $derived(snapshot?.plugins.some(p => p.id === 'i18n' && p.enabled && !p.problem) ?? false);
</script>

<!-- §2.1, §3.6: keep the instance overview and tabs above the scrolling panel, with save in the footer. -->
<div class="form-page" class:instance-form={!!instance}>
  {#if instance}
    <div class="instance-header">
      <section class="instance-overview" aria-labelledby="instance-overview-title">
        <span class="instance-emblem"><InstanceIcon {instance} custom={snapshot?.instanceIcons?.[instance.id]} /></span>
        <div class="instance-identity">
          <h2 id="instance-overview-title" title={instance.name}>{instance.name}</h2>
          <p>Minecraft {instance.gameVersion} <span aria-hidden="true">·</span> {instanceLoaderLabel(instance)}</p>
        </div>
        <span class="instance-status" class:installed={instance.isInstalled}>
          <Icon name={instance.isInstalled ? 'circle-check' : 'clock'} size={15} />
          {instance.isInstalled ? t("已安装") : t("尚未安装完成")}
        </span>
      </section>
      <div class="segmented instance-tabs" role="tablist" aria-label={t("版本设置")}>
        {#each instanceTabs as tab, index (tab.id)}
          <button data-h-part="button" type="button" role="tab" id={`${formId}-tab-${tab.id}`} aria-controls={`${formId}-panel-${tab.id}`}
                  disabled={!tab.enabled} class:selected={instanceTab === tab.id} aria-selected={instanceTab === tab.id} tabindex={instanceTab === tab.id ? 0 : -1}
                  onclick={() => selectInstanceTab(tab.id)} onkeydown={event => onTabKeydown(event, index)}>
            <Icon name={tab.icon} size={17} />{tab.label}
          </button>
        {/each}
      </div>
    </div>
    <div class="scroll-pane form-scroll" role="tabpanel" id={`${formId}-panel-general`}
         aria-labelledby={`${formId}-tab-general`} hidden={instanceTab !== 'general'}>
      <section data-h-part="card" class="instance-panel" aria-labelledby={`${formId}-runtime-heading`}>
        <h2 id={`${formId}-runtime-heading`}>{t("启动配置")}</h2>
        <p class="caption">{t("这些设置只应用于当前实例。")}</p>
        <div class="instance-field">{@render javaEditor()}</div>
        <div class="instance-field">{@render memoryEditor()}</div>
        {@render jvmEditor()}
      </section>
      <section data-h-part="card" class="instance-panel" aria-labelledby={`${formId}-management-heading`}>
        <h2 id={`${formId}-management-heading`}>{t("实例管理")}</h2>
        <div class="instance-field">
          <label class="section-label" for="instance-name">{t("实例名称")}</label>
          <div class="input-action"><input data-h-part="input-shell" id="instance-name" maxlength="64" bind:value={instanceName} /><button data-h-part="button" disabled={busy} onclick={() => operate('renameInstance', { id: instance.id, name: instanceName })}>{t("重命名")}</button></div>
        </div>
        <div class="instance-actions">
          <button data-h-part="button" class="instance-action" disabled={busy} onclick={repair} aria-describedby="instance-repair-note">
            <Icon name="download" size={17} /><span>{t("下载或修复游戏文件")}<small id="instance-repair-note">{t("只下载缺失或损坏的文件。")}</small></span><Icon name="chevron" size={15} />
          </button>
          <button data-h-part="button" class="instance-action" disabled={busy} onclick={() => operate('openDirectory', { id: instance.id })}>
            <Icon name="folder" size={17} /><span>{t("打开实例目录")}</span><Icon name="chevron" size={15} />
          </button>
        </div>
      </section>
      <details class="instance-danger">
        <summary><Icon name="trash" size={16} />{t("删除实例")}<Icon name="chevron" size={15} /></summary>
        <p class="caption">{t("确认后删除版本目录及其中的文件。共享目录中的存档、模组和资源会保留。")}</p>
        <button data-h-part="button" data-h-variant="danger" class="danger" disabled={busy} onclick={ondelete}>{t("删除此实例")}</button>
      </details>
    </div>
    <div class="instance-tool-panel" role="tabpanel" id={`${formId}-panel-mods`}
         aria-labelledby={`${formId}-tab-mods`} hidden={instanceTab !== 'mods'}>
      {#if openedTools.mods && mods}{@render mods(instance)}{/if}
    </div>
    {#if canExport && exportPack}
      <div class="instance-tool-panel" role="tabpanel" id={`${formId}-panel-packExport`}
           aria-labelledby={`${formId}-tab-packExport`} hidden={instanceTab !== 'packExport'}>
        {#if openedTools.packExport}{@render exportPack(instance)}{/if}
      </div>
    {/if}
    {#each pluginTabs as tab (tab.key)}
      {#if instanceTab === tab.key}
        <div class="scroll-pane form-scroll" role="tabpanel" id={`${formId}-panel-${tab.key}`} aria-labelledby={`${formId}-tab-${tab.key}`}>
          <PluginPanel pluginId={tab.pluginId} panelId={tab.panel!} context={{ instanceId: instance.id }} locale={snapshot?.settings.locale} height={400} onerror={message => error = message} />
          {#if error}<p class="issue" role="alert">{error}</p>{/if}
        </div>
      {/if}
    {/each}
  {:else}
    <div class="scroll-pane form-scroll">
      <PluginSlot {snapshot} location="settings.sections" {onchange} />
      <label class="section-label" for="launcher-language">{t("语言 / Language")}</label>
      <select data-h-part="input-shell" id="launcher-language" bind:value={locale} disabled={busy || !languageAvailable}>
        {#each $i18n.languages as language}<option value={language.locale}>{language.name}</option>{/each}
      </select>
      <p class="caption">{t("保存后切换启动器语言，并设置游戏的默认语言；游戏内手动选择的语言会保留。")}</p>
      <h2 class="section-label">{t("账号")}</h2>
      <div class="field-row"><span>{snapshot?.account.playerName ?? (snapshot?.account.mode === 'offline' ? t("未登录离线账号") : t("未登录微软账号"))}{snapshot?.account.mode === 'offline' ? t(" · 离线模式") : ''}</span>{#if snapshot?.account.status === 'signedIn'}<button data-h-part="button" disabled={busy} onclick={() => operate('signOut')}>{t("退出登录")}</button>{:else}<button data-h-part="button" disabled={busy || snapshot?.account.status === 'signingIn'} onclick={onlogin}>{t("登录")}</button>{/if}</div>
      <p class="caption">{snapshot?.account.mode === 'offline' ? t("退出离线身份不会影响微软登录凭据和游戏数据。") : t("退出登录只删除本机凭据，不删除游戏数据。微软凭据由当前 Windows 用户的 DPAPI 保护。")}</p>
      <button data-h-part="button" class="full-width tall" onclick={onprofiles}>{t("管理账号与游戏名")}</button>
      <label class="section-label" for="download-source">{t("游戏下载源")}</label>
      <select data-h-part="input-shell" id="download-source" bind:value={source}>{#each snapshot?.plugins.filter(p => p.enabled && !p.problem) ?? [] as plugin}{#each (plugin.providers ?? []).filter(p => p.kind === 'downloadSource') as provider}<option value={plugin.id === 'minecraft.download' ? provider.id : plugin.id + ':' + provider.id}>{provider.title}</option>{/each}{/each}</select>
      <p class="caption">{t("保存后用于新的下载请求。")}</p>
      <label class="section-label checkbox-row"><input data-h-part="input-shell" type="checkbox" bind:checked={useModMirror} disabled={busy} aria-describedby="mod-mirror-note" />{t("使用国内镜像下载模组")}</label>
      <p id="mod-mirror-note" class="caption">{t("默认启用 MCIM，镜像失败后自动回退源站；关闭后模组查询与下载统一使用源站。保存后用于新的请求。")}</p>
      {@render javaEditor()}
      {@render memoryEditor()}
      <label class="section-label" for="game-window">{t("游戏窗口")}</label>
      <select data-h-part="input-shell" id="game-window" bind:value={windowMode}><option value="default">{t("游戏默认")}</option><option value="custom">{t("自定义尺寸")}</option><option value="fullscreen">{t("全屏")}</option></select>
      {#if windowMode === 'custom'}<div class="window-size" in:entrance><input data-h-part="input-shell" type="number" aria-label={t("窗口宽度")} min="320" max="7680" bind:value={windowWidth} /><span>×</span><input data-h-part="input-shell" type="number" aria-label={t("窗口高度")} min="240" max="4320" bind:value={windowHeight} /></div>{/if}
      <label class="section-label" for="after-launch">{t("启动后")}</label>
      <select data-h-part="input-shell" id="after-launch" bind:value={afterLaunch}><option value="minimize">{t("最小化启动器，游戏退出后恢复")}</option><option value="keep">{t("保持启动器窗口")}</option></select>
      <h2 class="section-label">{t("数据目录")}</h2>
      <div class="field-row"><span class="path-label">{snapshot?.dataDirectory ?? t("尚未读取")}</span><button data-h-part="button" disabled={busy} onclick={() => operate('changeDataDirectory')}>{t("更改…")}</button></div>
      <p class="caption">{t("更改位置不会移动或删除已有数据。")}</p>
      <button data-h-part="button" class="text-button" onclick={() => operate('openDirectory')}>{t("打开数据目录")}</button><p class="caption directory-caption">{t("游戏文件目录")}</p><p class="path-label">{snapshot?.gameDirectory}</p><button data-h-part="button" class="full-width tall" disabled={busy} onclick={() => operate('changeGameDirectory')}>{t("选择游戏目录…")}</button><p class="caption">{t("可直接选择已有的 .minecraft，复用版本、依赖和资源。")}</p>
      <label class="section-label" for="directory-isolation">{t("版本隔离")}</label>
      <select data-h-part="input-shell" id="directory-isolation" bind:value={isolation}><option value="auto">{t("自动识别")}</option><option value="shared">{t("全部使用共享目录")}</option><option value="isolated">{t("所有版本独立目录")}</option></select>
      <p class="caption">{t("对所有版本生效。自动模式将模组版本或已有独立游戏文件的版本放在各自目录，其余使用共享目录。更改后需保存。")}</p>
      <button data-h-part="button" class="full-width tall legacy-import" onclick={onimport}>{t("导入旧版数据…")}</button><p class="caption">{t("先预览导入内容。原目录与凭据不会被修改。")}</p>
      {#if legacyImport}<p class:issue={legacyImport.phase === 'failed'} class="caption" role="status">{t(legacyImport.message)}{legacyImport.total ? ` · ${(100 * legacyImport.completed / legacyImport.total).toFixed(0)}%` : ''}</p>{#if taskActive(legacyImport) && legacyImport.phase !== 'committing'}<button data-h-part="button" class="text-button" onclick={() => operate('cancelTask', { id: legacyImport.id })}>{t("取消导入")}</button>{/if}{/if}
      {@render jvmEditor()}
      <details class="expander diagnostics"><summary>{t("关于与界面诊断")}</summary>
        <dl><dt>{t("当前构建")}</dt><dd>{snapshot ? `${snapshot.version} · ${snapshot.stage}` : t("正在读取")}</dd><dt>{t("窗口材质")}</dt><dd>{t(snapshot?.appearance.reason ?? "正在读取")}</dd><dt>{t("原生圆角")}</dt><dd>{snapshot?.appearance.rounded ? t("已验证窗口区域裁剪") : t("未启用")}</dd><dt>{t("系统 WebView2")}</dt><dd>{snapshot?.webviewVersion || t("正在读取")}</dd><dt>{t("界面字体")}</dt><dd>{t("内置 MiSans / 系统备用字体")}</dd></dl>
      </details>
    </div>
  {/if}
  {#if !instance || instanceTab === 'general'}
    <footer class="form-footer"><div class="form-feedback">{#if error}<p class="issue" role="alert" in:entrance use:animateText={error}>{error}</p>{:else if message}<p class="caption" role="status" in:entrance use:animateText={message}>{message}</p>{:else if snapshot?.notice}<p class="issue">{t(snapshot.notice)}</p>{/if}</div><button data-h-part="button" class="save-button" class:primary={!!instance} disabled={busy || !snapshot || snapshot.readOnly} onclick={save}>{busy ? t("正在保存…") : t("保存")}</button></footer>
  {/if}
</div>

{#snippet javaEditor()}
  <label class="section-label" for="java-choice">Java</label>
  <select data-h-part="input-shell" id="java-choice" bind:value={java}>
    <option value="">{instance ? t("跟随启动器设置") : t("自动选择合适的 Java")}</option>{#if instance}<option value="automatic">{t("自动选择合适的 Java")}</option>{/if}
    {#if java && java !== 'automatic' && !snapshot?.javaRuntimes.some(j => j.path === java)}<option value={java}>{java}</option>{/if}{#each snapshot?.javaRuntimes ?? [] as runtime}<option value={runtime.path}>Java {runtime.version} · {runtime.architecture} · {runtime.path}</option>{/each}
  </select>
  <div class="inline-actions java-actions"><button data-h-part="button" disabled={busy} onclick={() => operate('javaScan')}>{t("重新扫描")}</button><button data-h-part="button" disabled={busy} onclick={chooseJava}>{t("手动选择…")}</button></div>
{/snippet}

<style>
  .instance-header { flex-shrink: 0; margin: 12px 24px 12px; }
  .instance-overview { display: flex; flex-wrap: wrap; align-items: center; gap: 14px; padding: 16px; margin-bottom: 14px; border-radius: 12px; background: var(--h-surface-sidebar); }
  .instance-emblem { display: grid; place-items: center; flex: 0 0 48px; height: 48px; border-radius: 10px; background: var(--h-selection); color: var(--h-text-primary); }
  .instance-identity { flex: 1; min-width: 0; }
  .instance-identity h2 { font-size: calc(var(--h-font-size,16px) * 1.375); font-weight: 600; line-height: 1.25; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .instance-identity p { margin-top: 4px; font-size: calc(var(--h-font-size,16px) * 0.875); color: var(--secondary); overflow-wrap: anywhere; }
  .instance-status { display: inline-flex; align-items: center; gap: 5px; padding: 5px 9px; border-radius: 999px; background: var(--h-surface-card); color: var(--h-text-primary); font-size: calc(var(--h-font-size,16px) * 0.75); white-space: nowrap; }
  .instance-status.installed { background: var(--h-surface-card); color: var(--h-text-primary); }
  .instance-tabs { flex-wrap: wrap; }
  .instance-tabs button { display: inline-flex; align-items: center; gap: 8px; }
  .instance-tabs button.selected { font-weight: 600; }
  .instance-tool-panel:not([hidden]) { display: flex; flex: 1; min-height: 0; flex-direction: column; }
  .instance-panel, .instance-danger { min-width: 0; padding: 16px; border: 1px solid var(--stroke); border-radius: 12px; }
  .instance-panel h2 { font-size: calc(var(--h-font-size,16px) * 1.0625); font-weight: 600; }
  .instance-panel + .instance-panel { margin-top: 12px; }
  .instance-field { margin-top: 14px; }
  .instance-field :global(.section-label) { margin: 0 0 6px; font-size: calc(var(--h-font-size,16px) * 0.875); font-weight: 600; color: inherit; }
  .instance-field :global(.caption) { margin-top: 4px; }
  .instance-field :global(.java-actions) { flex-wrap: wrap; margin-top: 8px; }
  .instance-field :global(.java-actions button), .instance-field .input-action button { padding: 5px 10px; font-size: calc(var(--h-font-size,16px) * 0.8125); white-space: nowrap; }
  .instance-field :global(.memory-editor) { grid-template-columns: minmax(0, 1fr) 78px auto; }
  .instance-field :global(.memory-editor input[type='number']) { width: 100%; margin: 0; }
  .instance-panel :global(.expander) { margin-top: 14px; border-top: 1px solid var(--h-surface-card); padding-top: 12px; }
  .instance-actions { display: grid; gap: 2px; margin-top: 12px; }
  .instance-action { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 8px; align-items: center; min-height: 40px; padding: 7px 8px; border: 0; box-shadow: none; text-align: left; font-size: calc(var(--h-font-size,16px) * 0.875); }
  .instance-action small { display: block; color: var(--secondary); font-size: calc(var(--h-font-size,16px) * 0.75); margin-top: 3px; }
  .instance-danger { margin-top: 12px; padding: 12px 16px; }
  .instance-danger summary { display: grid; grid-template-columns: auto 1fr auto; align-items: center; gap: 8px; color: var(--secondary); font-size: calc(var(--h-font-size,16px) * 0.875); cursor: pointer; list-style: none; }
  .instance-danger summary::-webkit-details-marker { display: none; }
  .instance-danger[open] summary :global(svg:last-child) { transform: rotate(90deg); }
  .instance-danger button { margin-top: 8px; font-size: calc(var(--h-font-size,16px) * 0.875); }
  .instance-form .form-footer { margin-top: 0; padding-top: 9px; border-top: 1px solid var(--h-surface-card); }
</style>

{#snippet memoryEditor()}
  <h3 class="section-label">{t("最大内存")}</h3>
  <p class="caption">{t("物理内存")} {snapshot?.memory.totalMib ?? '—'} {t("MB · 建议")} {snapshot?.memory.recommendedMib ?? '—'} MB</p>
  {#if instance}<label class="checkbox-row"><input data-h-part="input-shell" type="checkbox" bind:checked={inheritMemory} />{t("跟随启动器设置（{0} MB）", snapshot?.settings.maxMemoryMib ?? 4096)}</label>{/if}
  <div class="memory-editor" class:control-disabled={instance && inheritMemory}>
    <input data-h-part="input-shell" type="range" min="512" max={snapshot?.memory.totalMib ?? 65536} step="256" bind:value={memory} disabled={!!instance && inheritMemory} aria-label={t("最大内存")} />
    <input data-h-part="input-shell" type="number" min="512" max={snapshot?.memory.totalMib ?? 65536} step="256" bind:value={memory} disabled={!!instance && inheritMemory} aria-label={t("最大内存（MB）")} /><span>MB</span>
  </div>
  <p class="caption">{t("按游戏版本与模组数量分配内存，并为系统保留足够空间。")}</p>
{/snippet}

{#snippet jvmEditor()}
  <details class="expander"><summary>{t("高级 JVM 参数")}</summary>
    {#if instance}<label class="checkbox-row"><input data-h-part="input-shell" type="checkbox" bind:checked={inheritJvm} />{t("跟随启动器设置")}</label>{/if}
    <input data-h-part="input-shell" aria-label={t("JVM 参数")} placeholder={t("默认留空")} bind:value={jvm} disabled={!!instance && inheritJvm} />
    <p class="caption">{t("不能设置身份、工作目录、classpath 或内存等由启动器管理的参数。")}</p>
  </details>
{/snippet}
