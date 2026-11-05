import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import './fonts.css';

document.addEventListener('contextmenu', event => {
 const target = event.target;
 if (target instanceof HTMLInputElement && ['text', 'search', 'number', 'password', 'email', 'url', 'tel'].includes(target.type)) return;
 if (target instanceof HTMLTextAreaElement || (target instanceof HTMLElement && target.isContentEditable)) return;
 event.preventDefault();
});

type Frontend = { id: string; digest:string; entry: string; styles: string[] };
async function start() {
 const frontends = await invoke<Frontend[]>('ui_bootstrap', { provider: 'ui.shell' });
 for (const frontend of frontends) for (const path of frontend.styles) {
  const link = document.createElement('link'); link.rel = 'stylesheet';
  link.href = `${convertFileSrc('', 'hydrogen-ui')}${frontend.id}/${path}`;
  document.head.append(link);
 }
 const shell = frontends.find(p => p.id === 'ui.shell');
 if (!shell) throw new Error('缺少界面入口。');
 const url = `${convertFileSrc('', 'hydrogen-ui')}${shell.id}/${shell.entry}`;
 await import(/* @vite-ignore */ url);
 const revision = JSON.stringify(frontends.map(p => [p.id,p.digest]));
 async function checkRevision() {
  try {
   const next = await invoke<Frontend[]>('ui_bootstrap', { provider: 'ui.shell' });
   if (JSON.stringify(next.map(p => [p.id, p.digest])) !== revision) { location.reload(); return; }
  } catch { location.reload(); return; }
  setTimeout(checkRevision, 3000);
 }
 setTimeout(checkRevision, 3000);
}
start().catch(error => { const root = document.getElementById('app')!;root.textContent = `界面插件加载失败：${error?.message ?? error}`;root.style.cssText = 'padding:24px;color:#fff;background:#222;font:14px sans-serif;white-space:pre-wrap'; });
