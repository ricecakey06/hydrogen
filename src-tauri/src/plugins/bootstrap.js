(() => {
  const manifest = JSON.parse(globalThis.__manifest);
  const declared = new Set(manifest.contributes.commands.map(c => c.id));
  const commands = new Map(), services = new Map(), events = new Map(), hooks = new Map();
  const disposers = new Set(), pending = new Map(), readyHandlers = [];
  const request = globalThis.__hostRequest;
  let next = 0;
  function call(method, params = null) {
    if (pending.size >= 8) return Promise.reject(new Error('HOST_REQUEST_LIMIT'));
    return new Promise((resolve, reject) => {
      const id = ++next, json = JSON.stringify(params);
      if (json.length > 262144) { reject(new Error('MESSAGE_LIMIT')); return; }
      pending.set(id, { resolve, reject });
      if (!request(id, method, json)) {
        pending.delete(id);
        reject(new Error('HOST_REQUEST_LIMIT'));
      }
    });
  }
  function register(map, id, handler) {
    if (typeof handler !== 'function' || map.has(id)) throw new Error('INVALID_REGISTRATION');
    map.set(id, handler);
    const dispose = () => { map.delete(id); disposers.delete(dispose); };
    disposers.add(dispose);
    return Object.freeze({ dispose });
  }
  const group = (name, methods) => Object.freeze(Object.fromEntries(methods.map(method => [method, (params = {}) => call(name + '.' + method, params)])));
  Object.defineProperty(globalThis, '__settle', { value(id, ok, json) {
    const pair = pending.get(id);
    if (!pair) return;
    pending.delete(id);
    const value = JSON.parse(json);
    if (ok) pair.resolve(value);
    else pair.reject(Object.assign(new Error(value.message), value));
  }});
  Object.defineProperty(globalThis, '__hasReady', { value: () => readyHandlers.length > 0 });
  async function start() { while (readyHandlers.length) await readyHandlers.shift()(); }
  Object.defineProperty(globalThis, '__invoke', { value: async (kind, id, params) => {
    if (kind !== 'hook') await start();
    const map = kind === 'command' ? commands : kind === 'service' ? services : hooks;
    const handler = map.get(id);
    if (!handler && kind === 'hook') return null;
    if (!handler) throw new Error('HANDLER_NOT_REGISTERED');
    return handler(params);
  }});
  Object.defineProperty(globalThis, '__emit', { value: async (event, params) => {
    await start();
    for (const handler of events.get(event) ?? []) {
      try { await handler(params); } catch (error) {
        try { await call('lifecycle.diagnostic', { message: `${event}: ${String(error?.message ?? error)}`.slice(0, 1000) }); } catch {}
      }
    }
  }});
  Object.defineProperty(globalThis, '__dispose', { value: async () => {
    for (const dispose of [...disposers]) { try { await dispose(); } catch {} }
    events.clear(); hooks.clear(); services.clear(); commands.clear();
  }});
  Object.defineProperty(globalThis, '__context', { value: Object.freeze({
    commands: Object.freeze({ register(id, handler) {
      if (!declared.has(id)) throw new Error('UNDECLARED_COMMAND');
      return register(commands, id, handler);
    }}),
    services: Object.freeze({
      call: params => call('services.call', params),
      list: () => call('services.list'),
      register(id, methods) {
        const declaration = manifest.provides.find(service => service.id === id);
        if (!declaration || Object.keys(methods).some(key => !declaration.methods.includes(key))) throw new Error('UNDECLARED_SERVICE');
        const handles = declaration.methods.map(method => register(services, id + '/' + method, methods[method]));
        return { dispose() { handles.forEach(handle => handle.dispose()); } };
      }
    }),
    instances: Object.freeze({
      ...group('instances', ['list', 'create', 'configure', 'rename', 'delete']),
      select: id => call('instances.select', { id }),
      repair: id => call('instances.repair', { id }),
      launch: id => call('instances.launch', { id }),
    }),
    accounts: group('accounts', ['list', 'login', 'addOffline', 'addProvider', 'select', 'delete', 'signOut']),
    java: group('java', ['list', 'scan']),
    resources: group('resources', ['sources', 'filters', 'search', 'versions', 'dependencies', 'install']),
    mods: group('mods', ['list', 'updates', 'update', 'enable', 'delete', 'rename']),
    modpacks: group('modpacks', ['directories', 'import', 'export']),
    settings: Object.freeze({ get: () => call('settings.get'), update: patch => call('settings.update', patch) }),
    storage: Object.freeze({ get: key => call('storage.get', { key }), set: (key, value) => call('storage.set', { key, value }), delete: key => call('storage.delete', { key }) }),
    http: group('http', ['request', 'download']),
    files: group('files', ['read', 'write', 'list', 'delete']),
    process: group('process', ['run', 'cancel']),
    tasks: Object.freeze({ list: () => call('tasks.list'), get: id => call('tasks.get', { id }), cancel: id => call('tasks.cancel', { id }) }),
    events: Object.freeze({ on(event, handler) {
      if (!manifest.permissions.includes('events') || typeof handler !== 'function' || !['instance.changed', 'task.updated', 'game.exited'].includes(event)) throw new Error('INVALID_EVENT');
      const handlers = events.get(event) ?? new Set(); handlers.add(handler); events.set(event, handlers);
      return { dispose() { handlers.delete(handler); } };
    }}),
    hooks: Object.freeze({ register(name, handler) {
      if (!manifest.permissions.includes('hooks') || !['beforeLaunch', 'beforeInstall'].includes(name)) throw new Error('INVALID_HOOK');
      return register(hooks, name, handler);
    }}),
    log: Object.freeze({ write: (level, message) => call('log.write', { level, message }) }),
    ui: Object.freeze({ notify: message => call('ui.notify', message), update: (id, state) => call('ui.update', { id, state }), navigate: id => call('ui.navigate', { id }), theme: () => call('ui.theme') }),
    onReady(handler) { if (typeof handler !== 'function') throw new Error('INVALID_HANDLER'); readyHandlers.push(handler); },
    onDispose(handler) { if (typeof handler !== 'function') throw new Error('INVALID_DISPOSER'); disposers.add(handler); },
  })});
  delete globalThis.__hostRequest;
  delete globalThis.__manifest;
})();
