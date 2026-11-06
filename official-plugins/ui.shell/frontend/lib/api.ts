import { t } from './i18n.ts';
import { invoke } from '@tauri-apps/api/core';

export interface Overrides { java: string | null; maxMemoryMib: number | null; jvmArguments: string | null }
export interface Components { kind?: string; loader: string; fabricApi: string | null }
export interface InstanceInfo { id: string; name: string; gameVersion: string; isInstalled: boolean; overrides: Overrides; components: Components | null; directoryName: string | null; loaderName: string }
export function instanceLoaderLabel(instance: InstanceInfo) {
  const components = instance.components;
  if (!components) return instance.loaderName || t("原版");
  const name = instance.loaderName || ({ fabric: 'Fabric', forge: 'Forge', neoforge: 'NeoForge', quilt: 'Quilt' } as Record<string,string>)[components.kind ?? 'fabric'] || components.kind;
  return [name, components.loader].filter(Boolean).join(' ');
}
export interface Settings { theme: { id: string; mode: 'light' | 'dark' | 'system'; material: 'auto' | 'opaque' }; locale: string; gameDirectory: string | null; isolateGameDirectory: boolean | null; java: string | null; maxMemoryMib: number; windowMode: string; windowWidth: number; windowHeight: number; afterLaunch: string; jvmArguments: string; downloadSource: string; useModMirror: boolean; offlineEnabled: boolean; offlinePlayerName: string; accountMode: string; microsoftClientId: string; modules: Record<string, boolean> }
export interface JavaRuntime { path: string; version: string; major: number; architecture: string }
export interface DownloadProgress { completed: number; total: number | null; completedFiles: number; totalFiles: number; activeFiles: string[] }
export interface CrashCause { id: string; title: string; summary: string; suggestions: string[]; evidence: string[] }
export interface CrashReport { id: string; createdAt: number; exitCode: number; instanceName: string; gameVersion: string; javaVersion: string; javaArchitecture: string; maxMemoryMib: number; logPath: string; causes: CrashCause[]; files: string[]; notes: string[] }
export interface Task { id: string; instanceId: string | null; instanceName: string | null; gameVersion: string | null; kind: string; phase: string; message: string; completed: number; total: number | null; download: DownloadProgress | null; sequence: number; startedAt: number; logPath: string | null; exitCode: number | null; crashReport?: CrashReport | null; crashError?: string | null }
export interface Profile { id: string; kind: string; name: string; uuid: string | null; skin?: string | null }
export interface Account { profiles: Profile[]; selectedProfileId: string | null; status: string; playerName: string | null; mode: string; offlineAvailable: boolean; detail: string | null; skin: string | null }
export interface GameVersion { id: string; releaseTime: string; versionType: 'release' | 'snapshot' }
export interface ModInfo { id: string; englishName: string; chineseName: string | null; version: string; summary: string; kind: string; fileName: string | null; sha1: string | null; enabled: boolean; duplicate: boolean; icon: string | null; problem: string | null }
export interface ModRef { fileName: string; sha1: string }
export interface UiContribution { id: string; location: string; title: string; panel: string | null; command: string | null; order: number; group: string; when: 'instanceSelected' | 'modsSelected' | 'taskActive' | null }
export interface PluginInfo {
  id: string; name: string; version: string; digest: string; permissions: string[];
  commands: { id: string; title: string }[];
  panels: { id: string; title: string; entry: string }[];
  views: UiContribution[]; menus: UiContribution[];
  diagnostics?: string[];
  providers: { id: string; kind: string; title: string; service: string; version: string }[];
  runtime: 'javascript' | 'native';
  description: string; origin: 'builtIn' | 'development' | 'installed'; required: boolean; enabled: boolean; problem: string; dependencies: (string | { id: string; version?: string })[];
}
export interface Snapshot { stage: string; version: string; instances: InstanceInfo[]; instanceIcons?: Record<string, string>; selectedInstanceId: string; settings: Settings; javaRuntimes: JavaRuntime[]; account: Account; tasks: Task[]; dataDirectory: string; gameDirectory: string; notice: string | null; readOnly: boolean; memory: { totalMib: number; recommendedMib: number }; plugins: PluginInfo[]; webviewVersion: string; appearance: { material: 'blur' | 'solid'; reason: string; rounded: boolean } }
export function action<T = void>(type: string, fields: Record<string, unknown> = {}): Promise<T> { return invoke<T>('launcher_action', { action: { type, ...fields } }); }
export function taskActive(task: Task) { return !['completed', 'failed', 'cancelled', 'exited'].includes(task.phase); }
export interface RunReport { pluginId: string; generation: number; result: unknown; notifications: string[]; heapBytes: number; registrations: number }
export interface PanelSession { sessionId: string; nonce: string; url: string; generation: number; appearance: 'inherit' | 'independent' }
export { invoke };
export function errorMessage(error: unknown): string {
  if (error && typeof error === 'object' && 'message' in error) return t(String(error.message));
  return t(String(error));
}
