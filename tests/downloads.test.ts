import assert from 'node:assert/strict';
import test from 'node:test';
import { downloadBytes, downloadItem, downloadJobs, downloadRetryable, downloadSummary, preparationSummary } from '../official-plugins/ui.shell/frontend/lib/downloads.ts';
import type { Task } from '../official-plugins/ui.shell/frontend/lib/api.ts';

function task(fields: Partial<Task> = {}): Task {
  return { id: 'job', instanceId: 'instance', instanceName: '测试实例', gameVersion: '1.20.1', kind: 'install', phase: 'downloading', message: '', completed: 1, total: 2,
    download: { completed: 512 * 1024, total: 1024 * 1024, completedFiles: 1, totalFiles: 2, activeFiles: ['libraries\\large.jar'] }, sequence: 1, startedAt: 1, logPath: null, exitCode: null, ...fields };
}
test('download queue includes active and failed game and resource downloads', () => {
  const active = task(), failed = task({ phase: 'failed' }), cancelled = task({ phase: 'cancelled' });
  const mod = task({ kind: 'modDownload' });
  assert.deepEqual(downloadJobs([active, task({ phase: 'completed' }), cancelled, failed, mod, task({ kind: 'modInstall' }), task({ kind: 'launch' }), task({ kind: 'login' }), task({ kind: 'legacyImport' })]), [active, cancelled, failed, mod]);
  assert.equal(downloadSummary([]).text, '没有进行中的下载');
  assert.equal(downloadSummary([failed]).text, '下载已停止，可重试未完成的任务');
});
test('installation retries remain available before and after a modpack creates its instance', () => {
  const active = task({ kind: 'modpackImport', phase: 'extracting', instanceId: null });
  const failed = task({ kind: 'modpackImport', phase: 'failed' });
  const retryable = task({ kind: 'modpackImport', phase: 'cancelled' });
  const earlyFailure = { ...failed, instanceId: null };
  const earlyCancel = { ...retryable, instanceId: null };
  assert.deepEqual(downloadJobs([active, { ...active, phase: 'completed' }, failed, retryable, earlyFailure, earlyCancel]), [active, failed, retryable, earlyFailure, earlyCancel]);
  assert.ok([failed, retryable, earlyFailure, earlyCancel, task({ phase: 'failed' }), task({ phase: 'cancelled' })].every(downloadRetryable));
  assert.equal(downloadRetryable(active), false);
  const modFailure = task({ kind: 'modDownload', phase: 'failed' });
  assert.equal(downloadRetryable(modFailure), false);
  assert.equal(downloadSummary([modFailure]).text, '下载已停止');
});
test('overall progress is weighted by bytes and becomes indeterminate while resolving', () => {
  const small = task({ download: { completed: 1, total: 10, completedFiles: 1, totalFiles: 1, activeFiles: [] } });
  const large = task({ download: { completed: 90, total: 90, completedFiles: 0, totalFiles: 100, activeFiles: [] } });
  assert.equal(downloadSummary([small, large]).percent, 91);
  assert.equal(downloadSummary([small, large]).text, '总体进度 91% · 2 个任务正在下载');
  assert.equal(downloadSummary([small, task({ phase: 'resolving', download: null })]).text, '2 个任务正在进行');
});
test('item display uses original byte units, state labels and active filenames', () => {
  const item = downloadItem(task());
  assert.equal(item.status, '正在下载 · 50%');
  assert.equal(item.detail, '512.0 KB / 1.0 MB · 1 / 2 个文件');
  assert.deepEqual(item.files, ['libraries\\large.jar']);
  assert.equal(downloadItem(task({ phase: 'resolving', download: null })).status, '正在获取版本信息');
  assert.equal(downloadItem(task({ download: { completed: 1024, total: null, completedFiles: 0, totalFiles: 1, activeFiles: [] } })).status, '正在获取资源索引');
  assert.deepEqual(downloadItem(task({ phase: 'failed' })).files, []);
  assert.equal(downloadBytes(1024 ** 3), '1.00 GB');
});

test('post-install verification labels the second pass and keeps its own file progress', () => {
  const verifying = task({ kind: 'modpackImport', phase: 'verifying', message: '正在校验并补齐游戏文件…' });
  const item = downloadItem(verifying);
  assert.equal(item.status, '正在校验并补齐游戏文件 · 50%');
  assert.equal(item.indeterminate, false);
  assert.equal(item.percent, 50);
  assert.equal(item.filesLabel, '正在校验的文件');
  assert.equal(item.progressLabel, '文件校验进度');
  assert.deepEqual(item.files, ['libraries\\large.jar']);
  assert.equal(downloadSummary([verifying]).text, '总体校验进度 50% · 1 个任务正在校验文件');
  const pending = { ...verifying, download: null };
  assert.equal(downloadItem(pending).status, '正在校验并补齐游戏文件…');
  assert.equal(downloadSummary([pending]).text, '1 个任务正在校验文件');
  assert.equal(downloadSummary([verifying, task()]).text, '总体进度 50% · 2 个任务正在进行');
});

test('modpack resolution keeps the importer message before a file plan exists', () => {
  const resolving = task({ kind: 'modpackImport', phase: 'resolving', download: null, message: '正在读取整合包清单…' });
  assert.equal(downloadItem(resolving).status, resolving.message);
  assert.equal(downloadItem(resolving).indeterminate, true);
});

test('loader processors and other non-file stages never display stale transfer details', () => {
  for (const kind of ['install', 'modpackImport']) {
    const processing = task({ kind, phase: 'processing', message: '正在处理 Forge / NeoForge 安装步骤 2/7…' });
    const item = downloadItem(processing);
    assert.equal(item.status, processing.message);
    assert.equal(item.indeterminate, true);
    assert.equal(item.percent, 0);
    assert.equal(item.detail, '');
    assert.deepEqual(item.files, []);
    assert.equal(downloadSummary([processing]).indeterminate, true);
    assert.equal(downloadSummary([processing]).text, '1 个任务正在进行');
    assert.equal(downloadSummary([processing, task()]).text, '2 个任务正在进行');
  }
  for (const phase of ['preparing', 'extracting', 'staging', 'committing']) {
    const item = downloadItem(task({ phase, message: '正在处理…' }));
    assert.equal(item.detail, '');
    assert.deepEqual(item.files, []);
  }
});

test('launch preparation shows file progress when a Fabric library has no declared size', () => {
  const preparing = task({ kind: 'launch', phase: 'preparing', download: {
    completed: 512, total: null, completedFiles: 4800, totalFiles: 4819, activeFiles: ['libraries\\fabric-loader.jar'],
  } });
  const summary = preparationSummary(preparing);
  assert.equal(summary.progress, 4800 / 4819);
  assert.equal(summary.text, '正在准备游戏文件 4800/4819（99%）');
  assert.match(summary.detail, /fabric-loader\.jar/);
  assert.equal(preparationSummary(undefined).progress, null);
  assert.equal(preparationSummary(task({ download: null })).text, '正在准备游戏文件');
  assert.equal(preparationSummary(task()).progress, 0.5);
});
