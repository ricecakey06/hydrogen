(() => {
  const declared = new Set(JSON.parse(globalThis.__declared));
  const commands = new Map();
  const pending = new Map();
  const request = globalThis.__hostRequest;
  let next = 0;
  function call(method, params = null) {
    if (pending.size >= 8) return Promise.reject(new Error('HOST_REQUEST_LIMIT'));
    return new Promise((resolve, reject) => {
      const id = ++next;
      pending.set(id, { resolve, reject });
      if (!request(id, method, JSON.stringify(params))) {
        pending.delete(id);
        reject(new Error('HOST_REQUEST_LIMIT'));
      }
    });
  }
  Object.defineProperty(globalThis, '__settle', { value(id, ok, json) {
    const pair = pending.get(id);
    if (!pair) return;
    pending.delete(id);
    const value = JSON.parse(json);
    if (ok) pair.resolve(value);
    else pair.reject(Object.assign(new Error(value.message), value));
  }});
  Object.defineProperty(globalThis, '__invoke', { value(id) {
    const handler = commands.get(id);
    if (!handler) throw new Error('COMMAND_NOT_REGISTERED');
    return handler();
  }});
  Object.defineProperty(globalThis, '__context', { value: Object.freeze({
    commands: Object.freeze({ register(id, handler) {
      if (!declared.has(id) || typeof handler !== 'function' || commands.has(id)) throw new Error('INVALID_REGISTRATION');
      commands.set(id, handler);
    }}),
    instances: Object.freeze({ list: () => call('instances.list') }),
    ui: Object.freeze({ notify: (message) => call('ui.notify', message) }),
  })});
  delete globalThis.__hostRequest;
  delete globalThis.__declared;
})();
