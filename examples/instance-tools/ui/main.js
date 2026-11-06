import { bridge } from './bridge.js';
const status = document.querySelector('#status'), repair = document.querySelector('#repair'), cancel = document.querySelector('#cancel'), progress = document.querySelector('#progress');
let taskId, subscription;
function context(value) { repair.disabled = !value.instanceId; document.querySelector('#instance').textContent = value.instanceId ? '当前实例：' + value.instanceId : '请在启动器中选择一个实例'; }
function failure(error) { status.textContent = (error.code ? error.code + '：' : '') + error.message; }
bridge.ready.then(context);
bridge.onContext(context);
document.querySelector('#count').onclick = async () => { try { status.textContent = '实例数量：' + (await bridge.call('show-count')).count; } catch (e) { failure(e); } };
repair.onclick = async () => {
  repair.disabled = true;
  try {
    const task = await bridge.call('repair', bridge.context); taskId = task.id; cancel.disabled = false;
    subscription?.dispose();
    subscription = bridge.tasks.watch(taskId, task => {
      status.textContent = task.message;
      const total = task.download?.total ?? task.total, done = task.download?.completed ?? task.completed;
      if (total) progress.value = Math.min(100, done / total * 100); else progress.removeAttribute('value');
      const complete = ['completed', 'failed', 'cancelled', 'exited'].includes(task.phase);
      cancel.disabled = complete || task.phase === 'committing';
      if (complete) context(bridge.context);
    }, failure);
  } catch (e) { failure(e); context(bridge.context); }
};
cancel.onclick = async () => { try { await bridge.tasks.cancel(taskId); } catch (e) { failure(e); } };
