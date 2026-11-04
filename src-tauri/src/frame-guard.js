(() => {
  if (window.top === window) return;
  // WebView2 injects initialization scripts into all frames, even when marked main-only.
  // Remove the native transport before any plugin UI script runs. Rust ACL and CSP remain authoritative.
  const facade = Object.freeze(Object.create(null));
  Object.defineProperty(window, 'chrome', { value: facade, writable: false, configurable: false });
})();
