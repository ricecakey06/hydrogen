<!-- Pack details and directory selection lead, with one primary footer action.
     Use the shell's neutral controls, 8–24px spacing, and charcoal primary button. -->
<script lang="ts">
  import { onMount } from 'svelte';
  import { t } from '../../../ui.shell/frontend/lib/i18n';
  import { action, errorMessage, taskActive, type InstanceInfo, type Snapshot } from '../../../ui.shell/frontend/lib/api';
  import { downloadBytes } from '../../../ui.shell/frontend/lib/downloads';
  import Icon from '../../../ui.shell/frontend/components/Icon.svelte';

  type Format = 'modrinth' | 'curseforge';
  type Directory = { path: string; files: number; size: number; selected: boolean };
  let { instance, snapshot, onchange }: {
    instance: InstanceInfo; snapshot: Snapshot | null; onchange: () => Promise<void>;
  } = $props();
  let format = $state<Format>('modrinth');
  let folders = $state<Directory[]>([]);
  let selection = $state<Set<string>>(new Set());
  let name = $state('');
  let version = $state('1.0.0');
  let query = $state('');
  let showNested = $state(false);
  let limit = $state(200);
  let bundleUnmatched = $state(false);
  let busy = $state(false);
  let loaded = $state(false);
  let error = $state('');
  let taskId = $state('');
  const task = $derived(snapshot?.tasks.find(item => item.id === taskId)
    ?? snapshot?.tasks.find(item => item.kind === 'modpackExport' && item.instanceId === instance.id && taskActive(item)));
  const running = $derived(busy || !!(task && taskActive(task)));
  const term = $derived(query.trim().toLowerCase());
  const matches = $derived(folders.filter(folder => term
    ? label(folder.path).toLowerCase().includes(term) : showNested || !folder.path.includes('/')));
  const selectedBytes = $derived(folders.reduce((sum, folder) => sum + (selection.has(folder.path) ? folder.size : 0), 0));
  const partiallySelected = $derived.by(() => {
    const parents = new Set<string>();
    for (const path of selection) {
      const parts = path.split('/');
      while (parts.length > 1) { parts.pop(); parents.add(parts.join('/')); }
    }
    return parents;
  });

  function label(path: string) { return path === '.' ? t("实例根目录文件") : path; }
  function inherited(path: string, selected: Set<string>) {
    const parts = path.split('/');
    while (parts.length > 1) { parts.pop(); if (selected.has(parts.join('/'))) return true; }
    return false;
  }
  onMount(() => {
    name = instance.name;
    void loadFolders();
  });
  async function loadFolders() {
    busy = true; error = '';
    try {
      folders = await action<Directory[]>('modpackDirectories', { id: instance.id });
      selection = new Set(folders.filter(folder => folder.selected).map(folder => folder.path));
      loaded = true;
    } catch (e) { error = errorMessage(e); }
    finally { busy = false; }
  }
  function toggle(path: string, selected: boolean) {
    const next = new Set(selection);
    if (selected) {
      for (const child of next) { if (child.startsWith(path + '/')) next.delete(child); }
      next.add(path);
    } else next.delete(path);
    selection = next;
  }
  async function submit() {
    busy = true; error = '';
    try {
      const id = await action<string | null>('modpackExport', { id: instance.id, options: {
        format, name, version, directories: [...selection], bundleUnmatched
      } });
      if (id) { taskId = id; await onchange(); }
    } catch (e) { error = errorMessage(e); }
    finally { busy = false; }
  }
</script>

<div class="pack-page">
  <div class="scroll-pane pack-body">
    <div class="pack-fields">
      <label>{t("整合包名称")}<input data-h-part="input-shell" maxlength="64" bind:value={name} disabled={running} /></label>
      <label>{t("整合包版本")}<input data-h-part="input-shell" maxlength="64" bind:value={version} disabled={running} /></label>
      <label>{t("整合包格式")}<select data-h-part="input-shell" bind:value={format} disabled={running}><option value="modrinth">Modrinth (.mrpack)</option><option value="curseforge">CurseForge (.zip)</option></select></label>
    </div>
    <section aria-labelledby="pack-folders-title">
      <div class="section-row"><h2 class="section-label" id="pack-folders-title">{t("导出目录")}</h2><span class="caption">{t("已选 {0} 个 · {1}", selection.size, downloadBytes(selectedBytes))}</span></div>
      <p class="caption">{t("已勾选模组和常见配置目录。选择目录会包含其中的全部文件和子目录。")}</p>
      {#if folders.length}
        <div class="pack-tools">
          <input data-h-part="input-shell" type="search" aria-label={t("筛选目录路径")} placeholder={t("筛选目录路径")} bind:value={query} oninput={() => limit = 200} />
          <label class="pack-check"><input data-h-part="input-shell" type="checkbox" bind:checked={showNested} onchange={() => limit = 200} />{t("显示子目录")}</label>
        </div>
        <div class="pack-folders">
          {#each matches.slice(0, limit) as folder (folder.path)}
            {@const included = inherited(folder.path, selection)}
            <label class="pack-check folder-row">
              <input data-h-part="input-shell" type="checkbox" checked={selection.has(folder.path) || included} indeterminate={partiallySelected.has(folder.path)} disabled={running || included} onchange={event => toggle(folder.path, event.currentTarget.checked)} />
              <Icon name="folder" size={17} /><span title={label(folder.path)}>{label(folder.path)}</span>
              <small class="caption">{t("{0} 个文件", folder.files)}</small>
            </label>
          {:else}
            <p class="caption">{t("没有匹配的目录。")}</p>
          {/each}
          {#if matches.length > limit}<button data-h-part="button" class="text-button" onclick={() => limit += 200}>{t("显示更多目录")}</button>{/if}
        </div>
      {:else if busy}<p class="caption" role="status">{t("正在扫描实例目录…")}</p>
      {:else if loaded}<p class="caption">{t("实例目录没有可导出的文件。")}</p>
      {:else}<button data-h-part="button" class="text-button" onclick={loadFolders}>{t("重试")}</button>{/if}
    </section>
    <label class="pack-check"><input data-h-part="input-shell" type="checkbox" bind:checked={bundleUnmatched} disabled={running} /><span>{t("将有再分发许可的未匹配模组作为覆盖文件打包")}</span></label>
  </div>
  <div class="pack-footer">
    {#if error || task?.phase === 'failed'}<p class="issue" role="alert">{error || t(task?.message ?? '')}</p>
    {:else if task?.phase === 'completed'}<p class="caption path-label" role="status">{t("已保存整合包：{0}", task.message)}</p>
    {:else if task && taskActive(task)}<p class="caption" role="status">{t(task.message)}</p>{/if}
    <button data-h-part="button" data-h-variant="primary" class="primary full-width" disabled={running || !name.trim() || !loaded || !version.trim()} onclick={submit}>{t("选择保存目录并导出")}</button>
  </div>
</div>

<style>
  .pack-page { min-height: 0; flex: 1; display: flex; flex-direction: column; gap: 12px; padding: 8px 24px 20px; }
  .pack-body { flex: 1; display: flex; flex-direction: column; gap: 16px; padding: 0 4px 8px 0; }
  label input:not([type='checkbox']), label select { margin-top: 4px; }
  .pack-fields { display: grid; grid-template-columns: repeat(auto-fit, minmax(120px, 1fr)); gap: 8px; }
  .pack-body label { font-size: calc(var(--h-font-size,16px) * 0.875); }
  .pack-tools { display: flex; align-items: center; gap: 12px; margin: 8px 0; }
  .pack-tools label { flex-shrink: 0; }
  .pack-folders { max-height: 208px; overflow: auto; border: 1px solid var(--stroke); border-radius: 8px; padding: 4px 8px; background: var(--h-surface-card); }
  .pack-check { display: flex; align-items: center; gap: 8px; min-height: 32px; overflow-wrap: anywhere; }
  .pack-check input { flex-shrink: 0; }
  .folder-row span { min-width: 0; flex: 1; }
  .folder-row small { flex-shrink: 0; }
  .pack-footer { display: grid; gap: 8px; }
  .pack-footer p { margin: 0; overflow-wrap: anywhere; }
</style>
