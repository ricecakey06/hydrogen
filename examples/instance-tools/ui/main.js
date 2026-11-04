(() => {
  let port;
  let next = 0;
  const result = document.getElementById('result');
  const checks = {};
  try { void parent.document.body; checks.parentDom = 'FAILED'; } catch { checks.parentDom = 'denied'; }
  try { void localStorage.length; checks.storage = 'FAILED'; } catch { checks.storage = 'denied'; }
  checks.tauriObject = typeof window.__TAURI_INTERNALS__;
  checks.nativeTransport = typeof window.__TAURI_INTERNALS__?.ipc;
  checks.webviewTransport = typeof window.chrome?.webview;
  if (window.__TAURI_INTERNALS__?.invoke) {
    Promise.race([
      window.__TAURI_INTERNALS__.invoke('app_snapshot').then(() => 'FAILED', () => 'denied'),
      new Promise(resolve => setTimeout(() => resolve('no transport response (1s)'), 1000)),
    ])
      .then(value => { checks.nativeCommand = value; })
      .finally(() => { document.getElementById('probe').textContent = JSON.stringify(checks, null, 2); });
  } else checks.nativeCommand = 'unavailable';
  fetch('http://ipc.localhost/app_snapshot', { method: 'POST', body: '{}' })
    .then(() => { checks.directIpc = 'FAILED'; })
    .catch(() => { checks.directIpc = 'blocked'; })
    .finally(() => { document.getElementById('probe').textContent = JSON.stringify(checks, null, 2); });
  window.addEventListener('message', (event) => {
    if (event.source !== parent) return;
    if (event.data?.type === 'hydrogen:hello' && typeof event.data.nonce === 'string') {
      parent.postMessage({ type: 'hydrogen:ready', nonce: event.data.nonce }, '*');
    }
    if (event.data?.type === 'hydrogen:port' && event.ports.length === 1 && !port) {
      port = event.ports[0];
      port.onmessage = (message) => {
        result.textContent = message.data.ok ? `共有 ${message.data.result.result.count} 个实例` : '调用失败';
      };
      result.textContent = '已连接 · 等待读取';
    }
  });
  document.getElementById('count').addEventListener('click', () => {
    if (port) port.postMessage({ id: ++next, command: 'show-count' });
  });
})();
