import assert from 'node:assert/strict';
import test from 'node:test';
import { downloadBytes, downloadItem, downloadJobs, downloadSummary, preparationSummary } from '../official-plugins/ui.shell/frontend/lib/downloads.ts';
import type { Task } from '../official-plugins/ui.shell/frontend/lib/api.ts';

function task(fields: Partial<Task> = {}): Task {
  return { id: 'job', instanceId: 'instance', instanceName: '测试实例', gameVersion: '1.20.1', kind: 'install', phase: 'downloading', message: '', completed: 1, total: 2,
    download: { completed: 512 * 1024, total: 1024 * 1024, completedFiles: 1, totalFiles: 2, activeFiles: ['libraries\\large.jar'] }, sequence: 1, startedAt: 1, logPath: null, exitCode: null, ...fields };
}
test('download queue matches the original active/failed install queue', () => {
  const active = task(), failed = task({ phase: 'failed' });
  assert.deepEqual(downloadJobs([active, task({ phase: 'completed' }), task({ phase: 'cancelled' }), failed, task({ kind: 'launch' }), task({ kind: 'login' }), task({ kind: 'legacyImport' })]), [active, failed]);
  assert.equal(downloadSummary([]).text, '没有进行中的下载');
  assert.equal(downloadSummary([failed]).text, '下载已停止，可重试未完成的任务');
});
test('overall progress is weighted by bytes and becomes indeterminate while resolving', () => {
  const small = task({ download: { completed: 1, total: 10, completedFiles: 1, totalFiles: 1, activeFiles: [] } });
  const large = task({ download: { completed: 90, total: 90, completedFiles: 0, totalFiles: 100, activeFiles: [] } });
  assert.equal(downloadSummary([small, large]).percent, 91);
  assert.equal(downloadSummary([small, large]).text, '总体进度 91% · 2 个任务正在下载');
  assert.equal(downloadSummary([small, task({ phase: 'resolving', download: null })]).text, '2 个任务正在准备');
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
