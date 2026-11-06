import { writable } from 'svelte/store';
import type { Snapshot, UiContribution } from './api';
export type ViewContext = { instanceId?: string; selectedModIds?: string[]; taskId?: string; taskActive?: boolean; [key: string]: unknown };
export type Contribution = UiContribution & { key: string; pluginId: string; digest: string; title: string; enabled: boolean; badge?: string };
export const uiState = writable<Record<string, { generation?: number; title?: string; badge?: string; visible?: boolean; enabled?: boolean }>>({});
export function contributions(snapshot: Snapshot | null, location: string, context: ViewContext = {}, state: Record<string, { generation?: number; title?: string; badge?: string; visible?: boolean; enabled?: boolean }> = {}): Contribution[] {
  return (snapshot?.plugins ?? []).filter(p => p.enabled && !p.problem && p.permissions.includes('ui.contribute')).flatMap(plugin =>
    [...(plugin.views ?? []), ...(plugin.menus ?? [])].map((item, index) => {
      const key = plugin.id + '/' + plugin.digest + '/' + (item.id || String(index));
      return { ...item, key, pluginId: plugin.id, digest: plugin.digest,
        title: item.title || plugin.commands.find(c => c.id === item.command)?.title || plugin.panels.find(p => p.id === item.panel)?.title || plugin.name,
        enabled: true, ...state[key] };
    }).filter(item => item.location === location && item.visible !== false
      && (!item.when || item.when === 'instanceSelected' && !!context.instanceId
        || item.when === 'modsSelected' && !!context.selectedModIds?.length || item.when === 'taskActive' && context.taskActive))
  ).sort((a,b) => a.group.localeCompare(b.group) || a.order - b.order || a.key.localeCompare(b.key));
}
