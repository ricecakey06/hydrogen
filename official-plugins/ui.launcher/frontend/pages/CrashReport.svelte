<!--
  Content order: primary = possible cause and practical next steps; secondary = log evidence;
  tertiary = exit/environment details. Primary action opens the log; export is secondary.
  Use the shell's neutral colors (--secondary, --stroke), 12/16/20/24 spacing, 12px radius,
  and 14/18/22px type. Establish hierarchy with spacing and weight before adding color.
-->
<script lang="ts">
  import { t } from '../../../ui.shell/frontend/lib/i18n';
  import { action, errorMessage, type Task } from '../../../ui.shell/frontend/lib/api';
  let { task, onhome }: { task: Task | undefined; onhome: () => void } = $props();
  let pending = $state<'log' | 'export' | null>(null);
  let error = $state('');
  let exported = $state('');
  const report = $derived(task?.crashReport);
  async function openLog() {
    if (!task || pending) return;
    pending = 'log'; error = '';
    try { await action('openLogs', { id: task.id }); }
    catch (e) { error = errorMessage(e); }
    finally { pending = null; }
  }
  async function exportReport() {
    if (!task || !report || pending) return;
    pending = 'export'; error = ''; exported = '';
    try {
      const result = await action<{ path: string } | null>('exportCrashReport', { id: task.id });
      if (result) exported = result.path;
    } catch (e) { error = errorMessage(e); }
    finally { pending = null; }
  }
</script>

<div class="crash-page">
  {#if task}
    <div class="crash-heading">
      <h2>{t("游戏意外退出")}</h2>
      <p class="caption">{report?.instanceName || task.instanceName || 'Minecraft'} {t("· 退出码")} {report?.exitCode ?? task.exitCode ?? t("未知")}</p>
      <p class="crash-description">{t("以下是根据本次日志推测的原因，可按建议逐项排查。")}</p>
    </div>
    <div class="crash-actions">
      <button data-h-part="button" data-h-variant="primary" class="primary" disabled={!!pending || !(report?.logPath || task.logPath)} onclick={openLog}>{pending === 'log' ? t("正在打开…") : t("打开崩溃日志")}</button>
      <button data-h-part="button" disabled={!!pending || !report} onclick={exportReport}>{pending === 'export' ? t("正在导出…") : t("导出崩溃报告")}</button>
    </div>
    {#if error}<p class="issue crash-feedback" role="alert">{error}</p>{/if}
    {#if exported}<p class="notice crash-feedback" role="status">{t("报告已导出到：")}<span class="path-label">{exported}</span></p>{/if}
    <div class="scroll-pane crash-scroll">
      {#if report}
        {#each report.causes as cause (cause.id)}
          <section data-h-part="card" class="crash-cause" aria-label={t(cause.title)}>
            <h3>{t(cause.title)}</h3>
            <p>{t(cause.summary)}</p>
            <ol>{#each cause.suggestions as suggestion}<li>{t(suggestion)}</li>{/each}</ol>
            {#if cause.evidence.length}<details><summary>{t("查看日志依据")}</summary><pre>{cause.evidence.join('\n')}</pre></details>{/if}
          </section>
        {/each}
        <details class="crash-environment"><summary>{t("运行环境与报告说明")}</summary>
          <p class="caption">Minecraft {report.gameVersion} · Java {report.javaVersion} {report.javaArchitecture} {t("· 最大内存")} {report.maxMemoryMib} MiB</p>
          <p class="caption">{t("报告包含分析结果及")} {report.files.length} {t("份日志快照。常见令牌和本机路径会被脱敏，分享前请检查模组输出的私人内容。")}</p>
          {#each report.notes as note}<p class="caption">{t(note)}</p>{/each}
        </details>
      {:else}
        <section data-h-part="card" class="crash-cause">
          <h3>{t("暂时无法生成分析报告")}</h3>
          <p>{t("仍可打开本次游戏日志，查看末尾异常信息。")}</p>
          <p class="caption">{t(task.crashError) || t("请在插件管理中启用“游戏崩溃分析”，下次异常退出时会自动分析。")}</p>
        </section>
      {/if}
    </div>
    <button data-h-part="button" class="text-button crash-back" onclick={onhome}>{t("返回启动页面")}</button>
  {:else}
    <div class="empty-state">{t("本次会话还没有可查看的崩溃记录。")}</div>
    <button data-h-part="button" onclick={onhome}>{t("返回启动页面")}</button>
  {/if}
</div>
