import type { InstanceInfo } from '../../../ui.shell/frontend/lib/api.ts';

export type InstanceIconKind = 'vanilla' | 'forge' | 'neoforge' | 'fabric';

export function instanceIconKind(instance: Pick<InstanceInfo, 'components' | 'loaderName'>): InstanceIconKind {
  if (instance.components) {
    // Records created before loader kinds were introduced used Fabric exclusively.
    const kind = instance.components.kind?.toLowerCase() || 'fabric';
    return kind === 'forge' || kind === 'neoforge' || kind === 'fabric' ? kind : 'vanilla';
  }
  const kind = instance.loaderName.trim().toLowerCase().match(/^(neoforge|forge|fabric)(?=$|[\s\d._+-])/)?.[1];
  return (kind as InstanceIconKind | undefined) ?? 'vanilla';
}

export function instanceIconSources(custom: string | undefined, themed: string | undefined, builtin: string): string[] {
  return [...new Set([custom, themed, builtin].filter((source): source is string => !!source))];
}
