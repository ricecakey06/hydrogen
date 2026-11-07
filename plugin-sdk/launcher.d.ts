/** Hydrogen JavaScript API v2. ES modules; no Node.js, DOM or package installation. */
export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export interface Disposable { dispose(): void }
export interface InstanceInfo { id: string; name: string; gameVersion: string; isInstalled: boolean }
export interface TaskHandle { id: string }
export interface Task extends TaskHandle { instanceId: string | null; kind: string; phase: string; message: string; completed: number; total: number | null; sequence: number; download?: { completed: number; total: number | null } | null }
export interface ServiceRequest { provider: string; service: string; version: string; method: string; params?: Json }
export interface Provider { id: string; pluginId: string; service: string; title: string; version: string }
export interface CommandContext { instanceId?: string; selectedModIds?: string[]; taskId?: string; [key: string]: unknown }
export interface PluginContext {
  commands: { register<P = CommandContext>(id: string, handler: (params: P) => unknown | Promise<unknown>): Disposable };
  services: {
    call<T = Json>(request: ServiceRequest): Promise<T>;
    list(): Promise<{ provider: string; service: string; method: string; version: string; access: 'direct' | 'nativeOnly'; maxMessageBytes: number; timeoutMs: number }[]>;
    register(id: string, methods: Record<string, (params: any) => unknown | Promise<unknown>>): Disposable;
  };
  instances: {
    list(): Promise<InstanceInfo[]>; select(id: string): Promise<void>; repair(id: string): Promise<TaskHandle>; launch(id: string): Promise<TaskHandle>;
    create(params: { version: string; name: string; components?: { kind: string; loader: string; fabricApi?: string | null } }): Promise<TaskHandle>;
    rename(params: { id: string; name: string }): Promise<void>;
    configure(params: { id: string; overrides: { java: string | null; maxMemoryMib: number | null; gcMode?: 'preferZgc' | 'preferGenerationalZgc' | 'g1' | 'tunedG1' | null; jvmArguments: string | null } }): Promise<void>;
    delete(params: { id: string; confirmation: string }): Promise<void>;
  };
  accounts: {
    list(): Promise<Json>; login(): Promise<TaskHandle>; addOffline(params: { name: string }): Promise<Json>; addProvider(params: { provider: string; name: string }): Promise<Json>;
    select(params: { id: string }): Promise<void>; delete(params: { id: string; confirmation: string }): Promise<void>; signOut(): Promise<void>;
  };
  java: { list(): Promise<{ path: string; version: string; major: number; architecture: string }[]>; scan(): Promise<TaskHandle> };
  resources: {
    sources(): Promise<Provider[]>; filters(params: { source?: string }): Promise<Json>;
    search(params: { query: string; gameVersion: string; category: string; loader: string; offset: number; source?: string }): Promise<Json>;
    versions(params: { id: string; project: string; source?: string }): Promise<Json>;
    dependencies(params: { project: string; versionId?: string; source?: string }): Promise<Json>;
    install(params: { id: string; project: string; versionId: string; source?: string }): Promise<TaskHandle>;
  };
  mods: {
    list(params: { id: string }): Promise<Json>; updates(params: { id: string; hashes: string[] }): Promise<Json>;
    update(params: { id: string; files: ModRef[] }): Promise<TaskHandle>;
    enable(params: { id: string; files: ModRef[]; enabled: boolean }): Promise<void>;
    delete(params: { id: string; files: ModRef[]; confirmed: boolean }): Promise<void>;
    rename(params: { id: string; file: ModRef; name: string }): Promise<void>;
  };
  modpacks: { directories(params: { id: string }): Promise<Json>; import(params: { path: string }): Promise<TaskHandle>; export(params: { id: string; path: string; options: Json }): Promise<TaskHandle> };
  settings: { get(): Promise<Record<string, Json>>; update(patch: Record<string, Json>): Promise<void> };
  storage: { get<T = Json>(key: string): Promise<T | null>; set(key: string, value: Json): Promise<void>; delete(key: string): Promise<void> };
  /** Paths are relative to plugin-data/<pluginId>, never arbitrary system paths. */
  files: { read(params: { path: string }): Promise<{ text: string }>; write(params: { path: string; text: string }): Promise<void>; list(params: { path: string }): Promise<string[]>; delete(params: { path: string }): Promise<void> };
  /** Exact domain names in manifest.scopes.http apply to redirects too. */
  http: { request(params: { url: string; method?: 'GET' | 'POST'; body?: string }): Promise<{ status: number; text: string }>; download(params: { url: string; path: string }): Promise<TaskHandle> };
  /** Executable must exactly match an absolute path in manifest.scopes.process. */
  process: { run(params: { program: string; args: string[] }): Promise<TaskHandle>; cancel(params: { id: string }): Promise<void> };
  tasks: { list(): Promise<Task[]>; get(id: string): Promise<Task>; cancel(id: string): Promise<void> };
  events: { on(event: 'instance.changed' | 'task.updated' | 'game.exited', handler: (params: Json) => void | Promise<void>): Disposable };
  hooks: { register(name: 'beforeLaunch' | 'beforeInstall', handler: (params: Json) => HookResult | Promise<HookResult>): Disposable };
  log: { write(level: 'info' | 'warn' | 'error', message: string): Promise<void> };
  ui: { notify(message: { message: string }): Promise<void>; update(id: string, state: { title?: string; badge?: string; enabled?: boolean; visible?: boolean }): Promise<void>; navigate(id: string): Promise<void>; theme(): Promise<{ theme: 'light' | 'dark'; id: string; mode: 'light' | 'dark' | 'system'; revision: number; highContrast: boolean; locale: string }> };
  onReady(handler: () => void | Promise<void>): void;
  onDispose(handler: () => void | Promise<void>): void;
}
export interface ModRef { fileName: string; sha1: string }
/** Patches apply only to this launch; beforeInstall permits deny only. Conflicting fields abort. */
export type HookResult = { deny?: string; patch?: { maxMemoryMib?: number; jvmArguments?: string } } | null;
