/** Hydrogen plugin API v1, P0 supported subset. No DOM, Node.js, fetch or native modules. */
export interface InstanceInfo {
  id: string;
  name: string;
  gameVersion: string;
  isInstalled: boolean;
}
export interface PluginContext {
  commands: { register(id: string, handler: () => unknown | Promise<unknown>): void };
  instances: { list(): Promise<InstanceInfo[]> };
  ui: { notify(message: { message: string }): Promise<void> };
}
