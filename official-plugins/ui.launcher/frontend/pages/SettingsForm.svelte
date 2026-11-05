<script lang="ts">
  import { t, i18n } from '../../../ui.shell/frontend/lib/i18n';
  import { action, errorMessage, taskActive, type InstanceInfo, type Snapshot, type Settings, type JavaRuntime } from '../../../ui.shell/frontend/lib/api';
  import { entrance, animateText } from '../../../ui.shell/frontend/lib/motion';
  let { instance, snapshot, onmods, onlogin, onprofiles, onchange, ondelete, onimport, onrepair }: { instance?: InstanceInfo; snapshot: Snapshot | null; onmods: () => void; onlogin: () => void; onprofiles: () => void; onchange: () => Promise<void>; ondelete: () => void; onimport: () => void; onrepair: (id: string) => void } = $props();
  let locale = $state('zh-CN');
  const legacyImport = $derived(snapshot?.tasks.find(task => task.kind === 'legacyImport'));
  let instanceName = $state('');
  let source = $state('official');
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
    const settings = snapshot.settings;
    locale = $i18n.languages.some(language => language.locale === settings.locale) ? settings.locale : $i18n.locale;
    instanceName = instance?.name ?? '';
    java = instance ? instance.overrides.java ?? '' : settings.java ?? '';
    memory = instance?.overrides.maxMemoryMib ?? settings.maxMemoryMib; inheritMemory = instance?.overrides.maxMemoryMib == null;
    jvm = instance?.overrides.jvmArguments ?? settings.jvmArguments; inheritJvm = instance?.overrides.jvmArguments == null;
    isolation = settings.isolateGameDirectory == null ? 'auto' : settings.isolateGameDirectory ? 'isolated' : 'shared';
    source = settings.downloadSource; windowMode = settings.windowMode; windowWidth = settings.windowWidth; windowHeight = settings.windowHeight; afterLaunch = settings.afterLaunch;
    error = ''; message = '';
  });
  async function operate(type: string, fields: Record<string,unknown> = {}) { busy = true; error = ''; message = ''; try { const result = await action<unknown>(type, fields); await onchange(); return result; } catch (e) { error = errorMessage(e); } finally { busy = false; } }
  async function chooseJava() { const runtime = await operate('javaChoose') as JavaRuntime | null | undefined; if (runtime) java = runtime.path; }
  function repair() { if (instance && !busy) onrepair(instance.id); }
  async function save() {
    if (!snapshot) return; busy = true; error = ''; message = '';
    try {
      if (instance) { await action('saveOverrides', { id: instance.id, overrides: { java: java || null, maxMemoryMib: inheritMemory ? null : Number(memory), jvmArguments: inheritJvm ? null : jvm } }); }
      else { const settings: Settings = { ...snapshot.settings, locale: languageAvailable ? locale : snapshot.settings.locale, isolateGameDirectory: isolation === 'auto' ? null : isolation === 'isolated', java: java || null, maxMemoryMib: Number(memory), jvmArguments: jvm, downloadSource: source, windowMode, windowWidth: Number(windowWidth), windowHeight: Number(windowHeight), afterLaunch }; await action('saveSettings', { settings }); }
      await onchange(); message = t("设置已保存");
    } catch (e) { error = errorMessage(e); } finally { busy = false; }
  }
  const languageAvailable = $derived(snapshot?.plugins.some(p => p.id === 'i18n' && p.enabled && !p.problem) ?? false);
</script>

<div class="form-page">
  <div class="scroll-pane form-scroll">
    {#if instance}
      <label class="section-label" for="instance-name">{t("实例名称")}</label>
      <div class="input-action"><input id="instance-name" maxlength="64" bind:value={instanceName} /><button disabled={busy} onclick={() => operate('renameInstance', { id: instance.id, name: instanceName })}>{t("重命名")}</button></div>
      <p class="caption">Minecraft {instance.gameVersion} · {instance.components ? `Fabric ${instance.components.loader}` : instance.loaderName || t("原版")} · {instance.isInstalled ? t("已安装") : t("尚未安装完成")}</p>
      <h2 class="section-label">{t("模组")}</h2>
      <button class="full-width tall" onclick={onmods}>{t("管理此实例的模组")}</button>
      <h2 class="section-label">{t("游戏文件")}</h2>
      <button class="full-width tall" disabled={busy} onclick={repair}>{t("下载或修复游戏文件")}</button>
      <p class="caption">{t("复用已校验的文件，只下载缺失或损坏的内容。")}</p><button class="text-button" onclick={() => operate('openDirectory', { id: instance.id })}>{t("打开实例目录")}</button>
    {:else}
      <label class="section-label" for="launcher-language">{t("语言 / Language")}</label>
      <select id="launcher-language" bind:value={locale} disabled={busy || !languageAvailable}>
        {#each $i18n.languages as language}<option value={language.locale}>{language.name}</option>{/each}
      </select>
      <p class="caption">{t("保存后切换启动器语言，并设置游戏的默认语言；游戏内手动选择的语言会保留。")}</p>
      <h2 class="section-label">{t("账号")}</h2>
      <div class="field-row"><span>{snapshot?.account.playerName ?? (snapshot?.account.mode === 'offline' ? t("未登录离线账号") : t("未登录微软账号"))}{snapshot?.account.mode === 'offline' ? t(" · 离线模式") : ''}</span>{#if snapshot?.account.status === 'signedIn'}<button disabled={busy} onclick={() => operate('signOut')}>{t("退出登录")}</button>{:else}<button disabled={busy || snapshot?.account.status === 'signingIn'} onclick={onlogin}>{t("登录")}</button>{/if}</div>
      <p class="caption">{snapshot?.account.mode === 'offline' ? t("退出离线身份不会影响微软登录凭据和游戏数据。") : t("退出登录只删除本机凭据，不删除游戏数据。微软凭据由当前 Windows 用户的 DPAPI 保护。")}</p>
      <button class="full-width tall" onclick={onprofiles}>{t("管理账号与游戏名")}</button>
      <label class="section-label" for="download-source">{t("游戏下载源")}</label>
      <select id="download-source" bind:value={source}><option value="official">{t("官方源（Mojang）")}</option><option value="bmclapi">BMCLAPI（bangbang93）</option></select>
      <p class="caption">{source === 'official' ? t("从 Mojang 官方服务下载游戏文件。保存后用于新的下载请求。") : t("使用 BMCLAPI 镜像。保存后用于新的下载请求。")}</p>
    {/if}

    <label class="section-label" for="java-choice">Java</label>
    <select id="java-choice" bind:value={java}>
      <option value="">{instance ? t("跟随启动器设置") : t("自动选择合适的 Java")}</option>{#if instance}<option value="automatic">{t("自动选择合适的 Java")}</option>{/if}
      {#if java && java !== 'automatic' && !snapshot?.javaRuntimes.some(j => j.path === java)}<option value={java}>{java}</option>{/if}{#each snapshot?.javaRuntimes ?? [] as runtime}<option value={runtime.path}>Java {runtime.version} · {runtime.architecture} · {runtime.path}</option>{/each}
    </select>
    <div class="inline-actions java-actions"><button disabled={busy} onclick={() => operate('javaScan')}>{t("重新扫描")}</button><button disabled={busy} onclick={chooseJava}>{t("手动选择…")}</button></div>
    <h2 class="section-label">{t("最大内存")}</h2><p class="caption">{t("物理内存")} {snapshot?.memory.totalMib ?? '—'} {t("MB · 建议")} {snapshot?.memory.recommendedMib ?? '—'} MB</p>
    {#if instance}<label class="checkbox-row"><input type="checkbox" bind:checked={inheritMemory} />{t("跟随启动器设置（{0} MB）", snapshot?.settings.maxMemoryMib ?? 4096)}</label>{/if}
    <div class="memory-editor" class:control-disabled={instance && inheritMemory}>
      <input type="range" min="512" max={snapshot?.memory.totalMib ?? 65536} step="256" bind:value={memory} disabled={!!instance && inheritMemory} aria-label={t("最大内存")} />
      <input type="number" min="512" max={snapshot?.memory.totalMib ?? 65536} step="256" bind:value={memory} disabled={!!instance && inheritMemory} aria-label={t("最大内存（MB）")} /><span>MB</span>
    </div>
    <p class="caption">{t("按游戏版本与模组数量分配内存，并为系统保留足够空间。")}</p>

    {#if !instance}
      <label class="section-label" for="game-window">{t("游戏窗口")}</label>
      <select id="game-window" bind:value={windowMode}><option value="default">{t("游戏默认")}</option><option value="custom">{t("自定义尺寸")}</option><option value="fullscreen">{t("全屏")}</option></select>
      {#if windowMode === 'custom'}<div class="window-size" in:entrance><input type="number" aria-label={t("窗口宽度")} min="320" max="7680" bind:value={windowWidth} /><span>×</span><input type="number" aria-label={t("窗口高度")} min="240" max="4320" bind:value={windowHeight} /></div>{/if}
      <label class="section-label" for="after-launch">{t("启动后")}</label>
      <select id="after-launch" bind:value={afterLaunch}><option value="minimize">{t("最小化启动器，游戏退出后恢复")}</option><option value="keep">{t("保持启动器窗口")}</option></select>
      <h2 class="section-label">{t("数据目录")}</h2>
      <div class="field-row"><span class="path-label">{snapshot?.dataDirectory ?? t("尚未读取")}</span><button disabled={busy} onclick={() => operate('changeDataDirectory')}>{t("更改…")}</button></div>
      <p class="caption">{t("更改位置不会移动或删除已有数据。")}</p>
      <button class="text-button" onclick={() => operate('openDirectory')}>{t("打开数据目录")}</button><p class="caption directory-caption">{t("游戏文件目录")}</p><p class="path-label">{snapshot?.gameDirectory}</p><button class="full-width tall" disabled={busy} onclick={() => operate('changeGameDirectory')}>{t("选择游戏目录…")}</button><p class="caption">{t("可直接选择已有的 .minecraft，复用版本、依赖和资源。")}</p>
      <label class="section-label" for="directory-isolation">{t("版本隔离")}</label>
      <select id="directory-isolation" bind:value={isolation}><option value="auto">{t("自动识别")}</option><option value="shared">{t("全部使用共享目录")}</option><option value="isolated">{t("所有版本独立目录")}</option></select>
      <p class="caption">{t("对所有版本生效。自动模式将模组版本或已有独立游戏文件的版本放在各自目录，其余使用共享目录。更改后需保存。")}</p>
      <button class="full-width tall legacy-import" onclick={onimport}>{t("导入旧版数据…")}</button><p class="caption">{t("先预览导入内容。原目录与凭据不会被修改。")}</p>
      {#if legacyImport}<p class:issue={legacyImport.phase === 'failed'} class="caption" role="status">{t(legacyImport.message)}{legacyImport.total ? ` · ${(100 * legacyImport.completed / legacyImport.total).toFixed(0)}%` : ''}</p>{#if taskActive(legacyImport) && legacyImport.phase !== 'committing'}<button class="text-button" onclick={() => operate('cancelTask', { id: legacyImport.id })}>{t("取消导入")}</button>{/if}{/if}
    {/if}

    <details class="expander"><summary>{t("高级 JVM 参数")}</summary>
      {#if instance}<label class="checkbox-row"><input type="checkbox" bind:checked={inheritJvm} />{t("跟随启动器设置")}</label>{/if}
      <input aria-label={t("JVM 参数")} placeholder={t("默认留空")} bind:value={jvm} disabled={!!instance && inheritJvm} />
      <p class="caption">{t("不能设置身份、工作目录、classpath 或内存等由启动器管理的参数。")}</p>
    </details>
    {#if instance}
      <h2 class="section-label">{t("删除实例")}</h2><button class="danger" disabled={busy} onclick={ondelete}>{t("删除此实例")}</button>
      <p class="caption">{t("确认后删除版本目录及其中的文件。共享目录中的存档、模组和资源会保留。")}</p>
    {:else}
      <details class="expander diagnostics"><summary>{t("关于与界面诊断")}</summary>
        <dl><dt>{t("当前构建")}</dt><dd>{snapshot ? `${snapshot.version} · ${snapshot.stage}` : t("正在读取")}</dd><dt>{t("窗口材质")}</dt><dd>{t(snapshot?.appearance.reason ?? "正在读取")}</dd><dt>{t("原生圆角")}</dt><dd>{snapshot?.appearance.rounded ? t("已验证窗口区域裁剪") : t("未启用")}</dd><dt>{t("系统 WebView2")}</dt><dd>{snapshot?.webviewVersion || t("正在读取")}</dd><dt>{t("界面字体")}</dt><dd>{t("MiSans / 系统中文字体")}</dd></dl>
      </details>
    {/if}
  </div>
  <footer class="form-footer"><div class="form-feedback">{#if error}<p class="issue" role="alert" in:entrance use:animateText={error}>{error}</p>{:else if message}<p class="caption" role="status" in:entrance use:animateText={message}>{message}</p>{:else if snapshot?.notice}<p class="issue">{t(snapshot.notice)}</p>{/if}</div><button class="save-button" disabled={busy || !snapshot || snapshot.readOnly} onclick={save}>{busy ? t("正在保存…") : t("保存")}</button></footer>
</div>
