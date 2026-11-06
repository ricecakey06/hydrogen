<script lang="ts">
  import PluginSlot from '../../../ui.shell/frontend/components/PluginSlot.svelte';
  import { t } from '../../../ui.shell/frontend/lib/i18n';
  import { tick } from 'svelte';
  import { action, errorMessage, taskActive, type Profile, type Snapshot } from '../../../ui.shell/frontend/lib/api';
  import Icon from '../../../ui.shell/frontend/components/Icon.svelte';
  import PlayerAvatar from '../../../ui.shell/frontend/components/PlayerAvatar.svelte';
  import { entrance } from '../../../ui.shell/frontend/lib/motion';

  // Primary: selected identity and one add action. Secondary: available profiles.
  // Reuse the shell's neutral colors, 24px page inset, 12/16px spacing and 8/12px radii (§2.8, §3.2).
  let { snapshot, onchange, ondelete }: { snapshot: Snapshot | null; onchange: () => Promise<void>; ondelete: (profile: Profile) => void } = $props();
  let adding = $state(false);
  let kind = $state<'microsoft' | 'offline'>('microsoft');
  let name = $state('');
  let pending = $state(false);
  let error = $state('');
  let message = $state('');
  let nameInput = $state<HTMLInputElement>();
  const account = $derived(snapshot?.account);
  const profiles = $derived(account?.profiles ?? []);
  const login = $derived(snapshot?.tasks.find(t => t.kind === 'login' && taskActive(t)));
  const lastLogin = $derived(snapshot?.tasks.find(t => t.kind === 'login'));
  const gameRunning = $derived(snapshot?.tasks.some(t => t.kind === 'launch' && taskActive(t)));
  const locked = $derived(pending || !!login || !!gameRunning || !snapshot || !!snapshot.readOnly);
  const microsoftAvailable = $derived(snapshot?.plugins.some(p => p.id === 'account.microsoft' && p.enabled && !p.problem));

  async function operate(type: string, fields: Record<string, unknown> = {}) {
    pending = true; error = ''; message = '';
    try { const result = await action<Profile>(type, fields); await onchange(); return result; }
    catch (e) { error = errorMessage(e); return null; }
    finally { pending = false; }
  }
  async function openAdd() {
    adding = true; error = ''; message = ''; name = '';
    kind = microsoftAvailable ? 'microsoft' : 'offline';
    await tick(); if (kind === 'offline') nameInput?.focus();
  }
  async function changeKind(next: 'microsoft' | 'offline') {
    kind = next; error = ''; await tick(); if (next === 'offline') nameInput?.focus();
  }
  async function add() {
    if (locked) return;
    if (kind === 'microsoft') {
      const result = await operate('profileAddMicrosoft');
      if (result) adding = false;
      return;
    }
    if (!/^[A-Za-z0-9_]{3,16}$/.test(name.trim())) {
      error = t("游戏名需要 3–16 个英文字母、数字或下划线。"); nameInput?.focus(); return;
    }
    const profile = await operate('profileAddOffline', { name: name.trim() });
    if (profile) { adding = false; name = ''; message = t("已添加 {0}", profile.name); }
  }
  async function select(profile: Profile) {
    if (locked) return;
    const selected = profile.id === account?.selectedProfileId;
    if (selected && account?.status === 'signedIn') return;
    await operate(selected ? 'signIn' : 'profileSelect', selected ? {} : { id: profile.id });
  }
</script>
{#each snapshot?.plugins.filter(p => p.enabled && !p.problem) ?? [] as plugin}
  {#each (plugin.providers ?? []).filter(p => p.kind === 'accountProvider' && !['account.microsoft','account.offline'].includes(plugin.id)) as provider}
    <button data-h-part="button" onclick={async () => { try { await action('profileAddProvider', { provider: plugin.id + ':' + provider.id, name: provider.title }); await onchange(); } catch (e) { error = errorMessage(e); } }}>{t("添加 {0}", provider.title)}</button>
  {/each}
{/each}

<PluginSlot {snapshot} location="profiles.actions" context={{ instanceId: snapshot?.selectedInstanceId }} onchange={onchange} />


<div class="profiles-page">
  <div class="profiles-toolbar">
    <div><h2>{t("游戏档案")}</h2><p class="caption">{t("选择启动游戏时使用的正版账号或离线玩家。")}</p></div>
    <button data-h-part="button" data-h-variant="primary" class="primary profile-add" disabled={locked || adding} onclick={openAdd}><Icon name="plus" size={18} />{t("添加档案")}</button>
  </div>
  {#if gameRunning}<p class="caption profile-feedback" role="status">{t("游戏运行或准备启动期间，暂时无法更改档案。")}</p>{/if}
  {#if snapshot?.readOnly}<p class="issue profile-feedback">{t("当前数据目录只读，暂时无法更改档案。")}</p>{/if}
  {#if account?.status === 'unavailable' && account.detail}<p class="issue profile-feedback" role="alert">{t(account.detail)}</p>{/if}
  {#if login && login.phase !== 'restoring'}
    <div class="profile-login" role="status"><Icon name="spinner" size={20} /><div><strong>{t("正在登录微软账号")}</strong><p class="caption">{t("请在系统浏览器中完成授权。")}</p></div><button data-h-part="button" class="text-button" disabled={pending} onclick={() => operate('cancelTask', { id: login.id })}>{t("取消登录")}</button></div>
  {:else if lastLogin?.phase === 'failed'}<p class="issue profile-feedback" role="alert">{t(lastLogin.message)}</p>{/if}
  {#if error}<p class="issue profile-feedback" role="alert">{error}</p>{/if}
  {#if message}<p class="notice profile-feedback" role="status">{message}</p>{/if}
  <div class="scroll-pane profiles-scroll">
    {#if adding}
      <form class="profile-form" onsubmit={event => { event.preventDefault(); void add(); }} in:entrance>
        <h3>{t("添加新档案")}</h3>
        <div class="segmented" role="group" aria-label={t("档案类型")}>
          <button data-h-part="button" type="button" class:selected={kind === 'microsoft'} aria-pressed={kind === 'microsoft'} disabled={locked || !microsoftAvailable} onclick={() => changeKind('microsoft')}>{t("正版账号")}</button>
          <button data-h-part="button" type="button" class:selected={kind === 'offline'} aria-pressed={kind === 'offline'} disabled={locked || !account?.offlineAvailable} onclick={() => changeKind('offline')}>{t("离线玩家")}</button>
        </div>
        {#if kind === 'offline'}
          <label class="section-label" for="profile-player-name">{t("游戏名")}</label>
          <input data-h-part="input-shell" id="profile-player-name" bind:this={nameInput} bind:value={name} placeholder={t("例如 Steve")} maxlength="16" autocomplete="off" spellcheck="false" disabled={locked} aria-describedby="profile-name-help" />
          <p class="caption" id="profile-name-help">{t("3–16 个英文字母、数字或下划线。不同游戏名会使用不同的离线玩家身份。")}</p>
        {:else}<p class="caption">{t("通过微软登录添加，验证成功后会出现在档案列表中。")}</p>{/if}
        {#if !account?.offlineAvailable}<p class="caption">{t("添加离线档案需要启用“离线启动”插件。")}</p>{/if}
        <div data-h-part="card" class="profile-form-actions"><button data-h-part="button" type="button" disabled={pending} onclick={() => { adding = false; error = ''; }}>{t("取消")}</button><button data-h-part="button" data-h-variant="primary" class="primary" disabled={locked || (kind === 'microsoft' ? !microsoftAvailable : !account?.offlineAvailable)} type="submit">{kind === 'microsoft' ? t("登录微软账号") : t("添加档案")}</button></div>
      </form>
    {/if}
    {#if profiles.length}
      <div class="profile-list" role="list" aria-label={t("已保存档案")}>
        {#each profiles as profile (profile.id)}
          {@const selected = profile.id === account?.selectedProfileId}
          {@const available = profile.kind === 'offline' ? account?.offlineAvailable : microsoftAvailable}
          <div data-h-part="card" class="profile-row" class:is-selected={selected} role="listitem">
            <button data-h-part="button" class="profile-select" class:profile-unavailable={!available} aria-label={`${selected ? (account?.status === 'signedOut' || account?.status === 'cached') ? t("重新登录") : t("当前档案") : t("使用档案")}：${profile.name}`} aria-pressed={selected} disabled={locked || !available} onclick={() => select(profile)}>
              <span class="profile-avatar"><PlayerAvatar skin={profile.skin} size={48} iconSize={28} iconWidth={1.5} /></span>
              <span class="profile-info"><strong>{profile.name}</strong><span>{profile.kind === 'microsoft' ? t("正版账号") : t("离线玩家")}{!available ? t(" · 插件未启用") : selected && (account?.status === 'signedOut' || account?.status === 'cached') ? t(" · 需要登录") : ''}</span></span>
              <span class="profile-choice">{#if selected}<Icon name="check" size={18} /><span>{(account?.status === 'signedOut' || account?.status === 'cached') ? t("重新登录") : t("正在使用")}</span>{:else}{t("使用此档案")}{/if}</span>
            </button>
            <button data-h-part="button" class="icon-button profile-delete" aria-label={t("删除档案：{0}", profile.name)} title={t("删除档案")} disabled={locked} onclick={() => ondelete(profile)}><Icon name="trash" size={19} width={1.7} /></button>
          </div>
        {/each}
      </div>
    {:else if !adding}
      <div class="profiles-empty"><span class="empty-plugin-icon"><Icon name="profiles" size={30} /></span><h3>{t("还没有游戏档案")}</h3><p class="caption">{t("点击“添加档案”，登录正版账号或创建离线玩家。")}</p></div>
    {/if}
  </div>
  <p class="caption profiles-note">{t("删除档案会移除其本机登录信息，游戏文件和存档会保留。")}</p>
</div>
