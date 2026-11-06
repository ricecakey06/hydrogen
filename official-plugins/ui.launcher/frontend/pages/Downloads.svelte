<script lang="ts">
  import PluginSlot from '../../../ui.shell/frontend/components/PluginSlot.svelte';
  import { t } from '../../../ui.shell/frontend/lib/i18n';
  import { action, errorMessage, taskActive, type Snapshot } from '../../../ui.shell/frontend/lib/api';
  import { downloadItem, downloadJobs, downloadSummary } from '../../../ui.shell/frontend/lib/downloads';
  import { animateText, entrance } from '../../../ui.shell/frontend/lib/motion';
  import DownloadProgress from '../../../ui.shell/frontend/components/DownloadProgress.svelte';
  let { snapshot, onchange, onhome, onrepair, startError }: { snapshot: Snapshot | null; onchange: () => Promise<void>; onhome: () => void; onrepair: (id: string) => Promise<void>; startError: string } = $props();
  let error = $state('');
  const jobs = $derived(downloadJobs(snapshot?.tasks ?? []));
  const overall = $derived(downloadSummary(jobs));
  async function retry(id: string) { error = ''; await onrepair(id); }
  async function operate(type: string, fields: Record<string, unknown>) { error = ''; try { await action(type, fields); await onchange(); } catch (e) { error = errorMessage(e); } }
</script>

<div class="downloads-page">
  <div class="download-overall" in:entrance>
    <p class="download-summary" role="status" use:animateText={overall.text}>{overall.text}</p>
    {#if jobs.length}<DownloadProgress percent={overall.percent} indeterminate={overall.indeterminate} overall />{/if}
    {#if error || startError}<p class="issue" role="alert">{error || startError}</p>{/if}
  </div>
  <div class="scroll-pane task-list">
    {#each jobs as task (task.id)}
      {@const item = downloadItem(task)}
      {@const instance = snapshot?.instances.find(instance => instance.id === task.instanceId)}
      {@const name = task.instanceName ?? instance?.name ?? task.gameVersion ?? ''}
      {@const version = task.gameVersion ?? instance?.gameVersion ?? ''}
      <section data-h-part="card" class="task-card" in:entrance|global>
        <PluginSlot {snapshot} location="downloads.taskActions" context={{ instanceId: task.instanceId ?? undefined, taskId: task.id, taskActive: taskActive(task) }} onchange={onchange} />
        <div class="task-heading"><h2 title={name}>{name}</h2>
          {#if taskActive(task)}<button data-h-part="button" class="text-button" disabled={task.phase === 'committing'} onclick={() => operate('cancelTask', { id: task.id })}>{t("取消")}</button>
          {:else if task.instanceId && (task.kind === 'install' && task.phase === 'failed' || task.kind === 'modpackImport' && ['failed', 'cancelled'].includes(task.phase))}<button data-h-part="button" onclick={() => task.instanceId && retry(task.instanceId)}>{t("重试下载")}</button>{/if}
        </div>
        {#if name !== version}<p class="caption">{version}</p>{/if}
        <p class="download-status" use:animateText={task.phase}>{task.kind === 'modpackImport' ? t("导入整合包") + ' · ' : task.kind === 'modUpdate' ? t("模组更新") + ' · ' : task.kind === 'modDownload' ? t("模组下载") + ' · ' : ''}{item.status}</p>
        <DownloadProgress percent={item.percent} indeterminate={item.indeterminate} />
        <p class="caption">{item.detail}</p>
        {#if item.files.length}<p class="caption">{t("正在处理的文件")}</p><div class="download-files">{#each item.files as file}<p>{file}</p>{/each}</div>{/if}
        {#if task.phase === 'failed'}<p class="issue" role="alert">{t(task.message)}</p>{/if}
      </section>
    {/each}
  </div>
  <button data-h-part="button" class="full-width" title={t("下载会继续进行，可从主页右上角返回此页")} onclick={onhome}>{t("返回主页")}</button>
</div>
