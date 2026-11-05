<script lang="ts">
  import { t, i18n } from '../../../ui.shell/frontend/lib/i18n';
  import { onDestroy } from 'svelte';
  import { action, errorMessage, taskActive, type InstanceInfo, type Snapshot, type ModInfo, type ModRef } from '../../../ui.shell/frontend/lib/api';
  import { entrance } from '../../../ui.shell/frontend/lib/motion';
  import { selectionFromBox } from '../../../ui.shell/frontend/lib/mod-selection';
  import Icon from '../../../ui.shell/frontend/components/Icon.svelte';
  let { instance, snapshot, onchange, onconfirm }: { instance?: InstanceInfo; snapshot: Snapshot | null; onchange: () => Promise<void>; onconfirm: (title: string, description: string, confirm: () => Promise<void>) => void } = $props();
  let disabledOnly = $state(false); let query = $state(''); let items = $state<ModInfo[]>([]);
  let selected = $state<string[]>([]); let error = $state(''); let notice = $state(''); let busy = $state(false); let loading = $state(false); let observed = $state(''); let page = $state(0);
  let editingName = $state(''); let newName = $state(''); let listElement: HTMLDivElement;
  let disposed = false, request = 0, lastDrag = -Infinity;
  let drag: { x: number; y: number; pointer: number; original: string[]; active: boolean } | null = null;
  let box = $state<{ x: number; y: number; w: number; h: number } | null>(null);
  const inUse = $derived(snapshot?.tasks.some(t => t.instanceId === instance?.id && taskActive(t)) ?? false);
  const filtered = $derived(items.filter(m => m.enabled !== disabledOnly && `${m.englishName} ${m.chineseName ?? ''} ${m.id} ${m.fileName}`.toLowerCase().includes(query.toLowerCase())));
  const displayed = $derived(filtered.slice(page * 50, (page + 1) * 50));
  const selectedItems = $derived(items.filter(m => m.fileName && selected.includes(m.fileName)));
  const canEnable = $derived(selectedItems.some(mod => !mod.enabled));
  const canDisable = $derived(selectedItems.some(mod => mod.enabled));
  $effect(() => { const key = `${instance?.id}/${snapshot?.dataDirectory}/${snapshot?.tasks.filter(t => t.instanceId === instance?.id && t.kind === 'modInstall').map(t => `${t.id}:${t.phase}`).join('/')}`; if (instance && key !== observed) { observed = key; void refresh(); } });
  onDestroy(() => { disposed = true; request++; });
  function reference(mod: ModInfo): ModRef { if (!mod.fileName || !mod.sha1) throw new Error(t("请刷新本地模组列表。")); return { fileName: mod.fileName, sha1: mod.sha1 }; }
  async function refresh() { if (!instance) return; const current = ++request; loading = true; error = ''; try { const result = await action<{ items: ModInfo[]; notice: string | null }>('modsList', { id: instance.id }); if (!disposed && current === request) { items = result.items; notice = result.notice ?? ''; selected = []; page = 0; } } catch (e) { if (!disposed) error = errorMessage(e); } finally { if (!disposed && current === request) loading = false; } }
  async function operate(type: string, fields: Record<string,unknown> = {}) { if (!instance) return; busy = true; error = ''; try { await action(type, { id: instance.id, ...fields }); await onchange(); await refresh(); } catch (e) { error = errorMessage(e); } finally { busy = false; } }
  function clearSelection(restoreFocus = false) { if (restoreFocus) listElement?.querySelector<HTMLElement>('.mod-card.selected .mod-row-main')?.focus(); selected = []; }
  function changeTab(disabled: boolean) { disabledOnly = disabled; page = 0; clearSelection(); }
  function selectionKeydown(event: KeyboardEvent) { if (event.key === 'Escape' && selected.length) { event.preventDefault(); event.stopPropagation(); clearSelection(true); } }
  function toggle(mod: ModInfo, event: MouseEvent) { if (!mod.fileName || (event.detail > 0 && performance.now() - lastDrag < 150)) return; selected = selected.includes(mod.fileName) ? selected.filter(n => n !== mod.fileName) : [...selected, mod.fileName]; }
  function removeSelected() { if (!instance || !selectedItems.length) return; const id = instance.id, files = selectedItems.map(reference); onconfirm(t("删除所选模组？"), t("将删除 {0} 个模组文件。其他文件和存档会保留。", files.length), async () => { await action('modsDelete', { id, files, confirmed: true }); await refresh(); }); }
  function dragStart(event: PointerEvent) { if (event.button !== 0 || !(event.target instanceof Element) || event.target.closest('input, .mod-actions, .mod-name-editor')) return; drag = { x: event.clientX, y: event.clientY, pointer: event.pointerId, original: [...selected], active: false }; }
  function dragMove(event: PointerEvent) {
    if (!drag || event.pointerId !== drag.pointer || !listElement) return;
    if (!drag.active && Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < 5) return;
    if (!drag.active) { drag.active = true; listElement.setPointerCapture(event.pointerId); }
    event.preventDefault();
    const left = Math.min(drag.x, event.clientX), top = Math.min(drag.y, event.clientY), right = Math.max(drag.x, event.clientX), bottom = Math.max(drag.y, event.clientY);
    const intersected: string[] = [];
    for (const card of listElement.querySelectorAll<HTMLElement>('[data-mod-file]')) { const rect = card.getBoundingClientRect(); if (rect.right >= left && rect.left <= right && rect.bottom >= top && rect.top <= bottom && card.dataset.modFile) intersected.push(card.dataset.modFile); }
    selected = selectionFromBox(drag.original, intersected); const rect = listElement.getBoundingClientRect(), scale = rect.width / listElement.clientWidth;
    box = { x: (left - rect.left) / scale, y: (top - rect.top) / scale + listElement.scrollTop, w: (right - left) / scale, h: (bottom - top) / scale };
  }
  function dragEnd(event: PointerEvent) { if (event.pointerId !== drag?.pointer) return; if (drag.active) { lastDrag = performance.now(); if (listElement.hasPointerCapture(event.pointerId)) listElement.releasePointerCapture(event.pointerId); } drag = null; box = null; }
</script>
<svelte:window onpointermove={dragMove} onpointerup={dragEnd} onpointercancel={dragEnd} />
<div class="mods-page" role="presentation" onkeydown={selectionKeydown}>
  <div class="mods-toolbar">
    <p class="caption">{instance?.name} · Minecraft {instance?.gameVersion}{instance?.components ? ` · Fabric ${instance.components.loader}` : instance?.loaderName ? ` · ${instance.loaderName}` : t(" · 原版")}</p>
    <div class="mod-tabs"><button class:active={!disabledOnly} onclick={() => changeTab(false)}>{t("已启用（")}{items.filter(m => m.enabled).length}）</button><button class:active={disabledOnly} onclick={() => changeTab(true)}>{t("已禁用（")}{items.filter(m => !m.enabled).length}）</button></div>
    <input class="mod-search" bind:value={query} oninput={() => { page = 0; clearSelection(); }} maxlength="100" placeholder={t("搜索本地模组的中英文名、ID 或文件名")} aria-label={t("搜索本地模组")} />
    <div class="inline-actions compact-actions"><button disabled={loading} onclick={() => refresh()}><Icon name="refresh" size={16} />{t("刷新")}</button><button disabled={busy || inUse} onclick={() => operate('modsImport')}><Icon name="plus" size={16} />{t("导入模组…")}</button><button onclick={() => operate('openMods')}><Icon name="folder" size={16} />{t("打开文件夹")}</button><button disabled={loading || !filtered.length} onclick={() => selected = filtered.flatMap(m => m.fileName ? [m.fileName] : [])}>{t("全选")}</button></div>
    <p class="caption small">{t("点击卡片选择，再次点击取消；按住左键框选可切换选中状态。可将模组文件拖入窗口。")}</p>
  </div>
  {#if loading}<p class="caption" role="status">{t("正在读取本地模组…")}</p>{/if}
  {#if error}<p class="issue" role="alert" in:entrance>{error}</p>{/if}{#if notice}<p class="caption">{t(notice)}</p>{/if}
  <div class="mod-list-stage">
  <div class="scroll-pane mod-list" class:has-selection={selectedItems.length > 0} bind:this={listElement} role="presentation" onpointerdown={dragStart}>
    {#each displayed as mod (mod.fileName ?? mod.id)}
      <article class="mod-card" class:selected={!!mod.fileName && selected.includes(mod.fileName)} data-mod-file={mod.fileName ?? undefined}>
        {#if mod.fileName && selected.includes(mod.fileName)}<span class="mod-selected-mark" aria-hidden="true"><Icon name="check" size={12} width={2.5} /></span>{/if}
        <button class="mod-row-main" aria-pressed={!!mod.fileName && selected.includes(mod.fileName)} onclick={event => toggle(mod, event)}>
          {#if mod.icon}<img src={mod.icon} alt="" width="42" height="42" loading="lazy" draggable="false" />{:else}<span class="mod-icon"><Icon name="puzzle" size={27} /></span>{/if}
          <span class="mod-card-text"><span class="mod-title">{$i18n.locale.startsWith('zh') ? mod.chineseName ?? mod.englishName : mod.englishName}</span>{#if mod.chineseName && mod.chineseName !== mod.englishName}<span class="caption">{mod.englishName}</span>{/if}<span class="caption">{mod.version} · {mod.kind}</span><span class="mod-summary">{mod.summary}</span><span class="plugin-id">{mod.fileName ?? mod.id}</span>{#if mod.problem}<span class="issue">{t(mod.problem)}</span>{/if}</span>
        </button>
        <div class="mod-actions"><button class="text-button" disabled={busy || inUse} onclick={() => operate('modsEnable', { files: [reference(mod)], enabled: !mod.enabled })}>{mod.enabled ? t("禁用") : t("启用")}</button><button class="text-button" onclick={() => { editingName = mod.fileName ?? ''; newName = mod.chineseName ?? ''; }}>{t("中文名")}</button></div>
        {#if editingName && editingName === mod.fileName}<div class="mod-name-editor input-action"><input aria-label={t("模组中文名")} maxlength="100" bind:value={newName} placeholder={t("中文名称；留空恢复默认")} /><button disabled={busy} onclick={async () => { await operate('modsName', { file: reference(mod), name: newName }); editingName = ''; }}>{t("保存")}</button></div>{/if}
      </article>
    {:else}{#if !loading}<div class="empty-state">{t("暂无模组，或没有匹配的结果")}</div>{/if}{/each}
    {#if box}<div class="mod-selection-box" style={`left:${box.x}px;top:${box.y}px;width:${box.w}px;height:${box.h}px`}></div>{/if}
  </div>
  {#if selectedItems.length}
    <div class="mod-selection-dock">
      <div class="mod-selection-toolbar" role="toolbar" aria-label={t("所选模组快捷操作")} in:entrance out:entrance>
        <p class="mod-selection-count" role="status"><span class="mod-selection-number">{selectedItems.length}</span><span>{t("项已选")}</span></p>
        <div class="mod-selection-actions">
          <button disabled={!canEnable || busy || loading || inUse} onclick={() => operate('modsEnable', { files: selectedItems.filter(mod => !mod.enabled).map(reference), enabled: true })}><Icon name="circle-check" size={18} />{t("启用")}</button>
          <button disabled={!canDisable || busy || loading || inUse} onclick={() => operate('modsEnable', { files: selectedItems.filter(mod => mod.enabled).map(reference), enabled: false })}><Icon name="circle-minus" size={18} />{t("禁用")}</button>
          <button class="selection-delete" disabled={busy || loading || inUse} onclick={removeSelected}><Icon name="trash" size={18} />{t("删除")}</button>
          <span class="mod-selection-divider" aria-hidden="true"></span>
          <button class="selection-close" aria-label={t("取消选择")} title={t("取消选择（Esc）")} onclick={() => clearSelection(true)}><Icon name="close" size={17} /></button>
        </div>
      </div>
    </div>
  {/if}
  </div>
  {#if filtered.length > 50 || inUse}<div class="mods-footer">
    {#if filtered.length > 50}<div class="field-row mod-pagination"><button disabled={page === 0} onclick={() => page--}>{t("上一页")}</button><span class="caption">{page + 1} / {Math.ceil(filtered.length / 50)}</span><button disabled={(page + 1) * 50 >= filtered.length} onclick={() => page++}>{t("下一页")}</button></div>{/if}
    {#if inUse}<p class="caption">{t("此实例正在安装、启动或运行，文件修改暂不可用。")}</p>{/if}
  </div>{/if}
</div>
