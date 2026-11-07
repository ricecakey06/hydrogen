import { t } from './i18n.ts';
import { taskActive, type Task } from './api.ts';

export function preparationSummary(task: Task | undefined) {
  const download = task?.download;
  const progress = download?.total ? download.completed / download.total
    : download?.totalFiles ? download.completedFiles / download.totalFiles : null;
  const fraction = progress == null ? null : Math.max(0, Math.min(1, progress));
  const text = !download ? t("正在准备游戏文件") : !download.totalFiles ? t("正在获取资源索引")
    : t("正在准备游戏文件 {0}/{1}（{2}%）",download.completedFiles,download.totalFiles,Math.floor((fraction ?? 0) * 100));
  const active = download?.activeFiles[0];
  return { progress: fraction, text, detail: active ? `${text} · ${active}` : text };
}

export function downloadRetryable(task: Task) {
  return ['failed', 'cancelled'].includes(task.phase)
    && (task.kind === 'modpackImport' || task.kind === 'install' && task.instanceId !== null);
}

export function downloadJobs(tasks: readonly Task[]) {
  return tasks.filter(task => ['install', 'modDownload', 'modUpdate', 'modpackImport'].includes(task.kind)
    && (taskActive(task) || task.phase === 'failed' || downloadRetryable(task)));
}
const filePhase = (task: Task) => ['downloading', 'verifying'].includes(task.phase);
export function downloadSummary(jobs: readonly Task[]) {
  const active = jobs.filter(taskActive);
  const indeterminate = active.some(task => !filePhase(task) || !(task.download?.total && task.download.total > 0));
  const total = jobs.reduce((sum, task) => sum + (task.download?.total ?? 0), 0);
  const completed = jobs.reduce((sum, task) => sum + Math.min(task.download?.completed ?? 0, task.download?.total ?? 0), 0);
  const percent = total > 0 ? Math.max(0, Math.min(100, 100 * completed / total)) : 0;
  let text = jobs.some(downloadRetryable) ? t("下载已停止，可重试未完成的任务") : jobs.length ? t("下载已停止") : t("没有进行中的下载");
  if (active.length) {
    if (active.every(task => task.phase === 'verifying')) {
      text = indeterminate ? t("{0} 个任务正在校验文件",active.length) : t("总体校验进度 {0}% · {1} 个任务正在校验文件",Math.round(percent),active.length);
    } else if (active.every(task => task.phase === 'downloading')) {
      text = indeterminate ? t("{0} 个任务正在下载",active.length) : t("总体进度 {0}% · {1} 个任务正在下载",Math.round(percent),active.length);
    } else if (active.every(task => ['queued', 'resolving', 'preparing'].includes(task.phase))) {
      text = t("{0} 个任务正在准备",active.length);
    } else {
      text = indeterminate ? t("{0} 个任务正在进行",active.length) : t("总体进度 {0}% · {1} 个任务正在进行",Math.round(percent),active.length);
    }
  }
  return { indeterminate, percent, text };
}
export function downloadBytes(value: number) {
  return value >= 1024 ** 3 ? `${(value / 1024 ** 3).toFixed(2)} GB` : value >= 1024 ** 2 ? `${(value / 1024 ** 2).toFixed(1)} MB` : `${(value / 1024).toFixed(1)} KB`;
}
export function downloadItem(task: Task) {
  const progress = task.download;
  const showFiles = filePhase(task);
  const indeterminate = taskActive(task) && (!showFiles || !(progress?.total && progress.total > 0));
  const percent = task.phase === 'completed' ? 100 : showFiles && progress?.total ? Math.max(0, Math.min(100, 100 * progress.completed / progress.total)) : 0;
  const status = task.phase === 'verifying' ? indeterminate ? t(task.message || "正在校验并补齐游戏文件…") : t("正在校验并补齐游戏文件 · {0}%",Math.round(percent))
    : task.kind === 'modpackImport' && taskActive(task) ? t(task.message || "正在处理…")
    : task.phase === 'staging' ? t("正在准备替换模组文件…") : task.kind === 'modUpdate' && task.phase === 'committing' ? t("正在提交模组文件…")
    : ['queued', 'resolving'].includes(task.phase) ? t("正在获取版本信息") : task.phase === 'completed' ? t("下载完成")
    : task.phase === 'cancelled' ? t("已取消") : task.phase === 'failed' ? t("下载失败")
    : !showFiles ? t(task.message || "正在处理…") : indeterminate ? t(task.message || "正在获取资源索引") : t("正在下载 · {0}%",Math.round(percent));
  const detail = showFiles && progress ? `${downloadBytes(progress.completed)} / ${progress.total == null ? t("待统计") : downloadBytes(progress.total)}`
    + (progress.totalFiles > 0 ? t(" · {0} / {1} 个文件",progress.completedFiles,progress.totalFiles) : '') : '';
  return { indeterminate, percent, status, detail, files: showFiles && taskActive(task) ? progress?.activeFiles ?? [] : [],
    filesLabel: task.phase === 'verifying' ? t("正在校验的文件") : t("正在处理的文件"),
    progressLabel: task.phase === 'verifying' ? t("文件校验进度") : t("任务进度") };
}
