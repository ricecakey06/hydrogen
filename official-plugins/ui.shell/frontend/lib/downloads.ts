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

export function downloadJobs(tasks: readonly Task[]) {
  return tasks.filter(task => ['install', 'modDownload', 'modUpdate', 'modpackImport'].includes(task.kind)
    && (taskActive(task) || task.phase === 'failed'
      || task.kind === 'modpackImport' && task.phase === 'cancelled' && task.instanceId));
}
export function downloadSummary(jobs: readonly Task[]) {
  const active = jobs.filter(taskActive).length;
  const indeterminate = jobs.some(task => taskActive(task) && !(task.download?.total && task.download.total > 0));
  const total = jobs.reduce((sum, task) => sum + (task.download?.total ?? 0), 0);
  const completed = jobs.reduce((sum, task) => sum + Math.min(task.download?.completed ?? 0, task.download?.total ?? 0), 0);
  const percent = total > 0 ? Math.max(0, Math.min(100, 100 * completed / total)) : 0;
  const text = active > 0 ? indeterminate ? t("{0} 个任务正在准备",active) : t("总体进度 {0}% · {1} 个任务正在下载",Math.round(percent),active)
    : jobs.length ? t("下载已停止，可重试未完成的任务") : t("没有进行中的下载");
  return { indeterminate, percent, text };
}
export function downloadBytes(value: number) {
  return value >= 1024 ** 3 ? `${(value / 1024 ** 3).toFixed(2)} GB` : value >= 1024 ** 2 ? `${(value / 1024 ** 2).toFixed(1)} MB` : `${(value / 1024).toFixed(1)} KB`;
}
export function downloadItem(task: Task) {
  const progress = task.download;
  const indeterminate = taskActive(task) && !(progress?.total && progress.total > 0);
  const percent = task.phase === 'completed' ? 100 : progress?.total ? Math.max(0, Math.min(100, 100 * progress.completed / progress.total)) : 0;
  const status = task.kind === 'modpackImport' && taskActive(task) ? t(task.message) : task.phase === 'staging' ? t("正在准备替换模组文件…") : task.kind === 'modUpdate' && task.phase === 'committing' ? t("正在提交模组文件…")
    : ['queued', 'resolving'].includes(task.phase) ? t("正在获取版本信息") : task.phase === 'completed' ? t("下载完成")
    : task.phase === 'cancelled' ? t("已取消") : task.phase === 'failed' ? t("下载失败") : indeterminate ? t("正在获取资源索引") : t("正在下载 · {0}%",Math.round(percent));
  const detail = progress ? `${downloadBytes(progress.completed)} / ${progress.total == null ? t("待统计") : downloadBytes(progress.total)}`
    + (progress.totalFiles > 0 ? t(" · {0} / {1} 个文件",progress.completedFiles,progress.totalFiles) : '') : '';
  return { indeterminate, percent, status, detail, files: taskActive(task) ? progress?.activeFiles ?? [] : [] };
}
