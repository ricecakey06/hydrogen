<script lang="ts">
  import { t } from '../../../ui.shell/frontend/lib/i18n';
  import { animateText, entrance } from '../../../ui.shell/frontend/lib/motion';
  import { taskActive, type Snapshot } from '../../../ui.shell/frontend/lib/api';
  import Icon from '../../../ui.shell/frontend/components/Icon.svelte';
  import PlayerAvatar from '../../../ui.shell/frontend/components/PlayerAvatar.svelte';
  import ProgressRing from '../../../ui.shell/frontend/components/ProgressRing.svelte';
  import { downloadJobs, downloadSummary, preparationSummary } from '../../../ui.shell/frontend/lib/downloads';
  let { snapshot, onlogin, oncancel, onlaunch, onversions, oninstance, onsettings, ondownloads, onlogs, oncrash }: {
    snapshot: Snapshot | null; onlogin: () => void; oncancel: (id: string) => void; onlaunch: () => void;
    onversions: () => void; oninstance: () => void; onsettings: () => void; ondownloads: () => void; onlogs: (id: string) => void; oncrash: (id: string) => void;
  } = $props();
  const selected = $derived(snapshot?.instances.find(i => i.id === snapshot.selectedInstanceId));
  const versionText = $derived(selected ? `${selected.name}${selected.name === selected.gameVersion ? '' : ` · ${selected.gameVersion}`}` : t("未选择版本"));
  const account = $derived(snapshot?.account);
  const loginTask = $derived(snapshot?.tasks.find(t => t.kind === 'login' && taskActive(t)));
  const login = $derived(loginTask?.phase === 'restoring' ? undefined : loginTask);
  const launch = $derived(snapshot?.tasks.find(t => t.kind === 'launch' && t.instanceId === selected?.id));
  const preparing = $derived(launch && taskActive(launch) && launch.phase !== 'running');
  const downloads = $derived(downloadJobs(snapshot?.tasks ?? []));
  const overall = $derived(downloadSummary(downloads));
  const downloading = $derived(downloads.find(task => task.instanceId === selected?.id && taskActive(task)));
  const downloadActionName = $derived(t("查看下载进度 · {0}", overall.text));
  const preparation = $derived(preparationSummary(launch));
  const launchProgress = $derived(launch?.phase === 'preparing' ? preparation.progress : null);
  const launchMessage = $derived(launch?.phase === 'preparing' ? preparation.text : launch?.phase === 'checking' ? t("正在检查环境") : launch?.message);
  const canLaunch = $derived(preparing || (selected?.isInstalled && account?.status === 'signedIn' && !loginTask && launch?.phase !== 'running' && !snapshot?.tasks.some(t => t.instanceId === selected?.id && taskActive(t))));
  const status = $derived(downloading ? t("游戏文件正在下载，可点击右上角查看进度") : launchMessage || account?.detail || '');
</script>
<div class="home">
  <div class="avatar" in:entrance|global><PlayerAvatar skin={account?.skin} size={88} /></div>
  {#if account?.playerName && !login}<div class="player-name" in:entrance|global={{ delay: 30 }}><span use:animateText={account.playerName}>{account.playerName}</span>{#if account?.mode === 'offline'}<button class="icon-button" aria-label={t("管理档案")} onclick={onsettings}><Icon name="settings" size={20} /></button>{:else if account.status === 'cached' && !loginTask}<button class="text-button" onclick={onlogin}>{t("重新登录")}</button>{/if}</div>
  {:else if login}<div class="signing-in" in:entrance><span>{t("正在浏览器中登录")}</span><button class="text-button" onclick={() => oncancel(login.id)}>{t("取消")}</button></div>
  {:else}<div class="signin-slot" in:entrance|global={{ delay: 30 }}><button class="signin" disabled={!snapshot || !!loginTask} onclick={onlogin}>{account?.mode === 'offline' ? t("登录离线账号") : t("登录微软账号")}</button></div>{/if}
  <p class="version" in:entrance|global={{ delay: 50 }}><span class="motion-text" use:animateText={versionText}>{versionText}</span></p>
  {#if downloads.length}<div class="downloads-slot"><ProgressRing progress={overall.indeterminate ? null : overall.percent / 100} size={50} /><button class="icon-button downloads-entry" aria-label={downloadActionName} title={downloadActionName} onclick={ondownloads}><Icon name="download" size={24} /></button></div>{/if}
  <div class="launch-slot" in:entrance|global={{ delay: 80 }}>{#if preparing}<ProgressRing progress={launchProgress} />{/if}<button class="launch" aria-label={preparing ? t("取消启动") : t("启动游戏")} disabled={!canLaunch} onclick={() => preparing && launch ? oncancel(launch.id) : onlaunch()}><Icon name={preparing ? 'stop' : 'play'} size={48} width={3.5} /></button></div>
  {#if status}<p class="home-status" class:failed={launch?.phase === 'failed' && !downloading} role="status" in:entrance><span class="motion-text" use:animateText={t(status)} title={t(launch?.phase === 'preparing' ? preparation.detail : status)}>{t(status)}</span>{#if launch?.phase === 'failed' && launch.logPath}{#if launch.crashReport || launch.crashError}<button class="text-button" onclick={() => oncrash(launch.id)}>{t("查看崩溃建议")}</button>{:else}<button class="text-button" onclick={() => onlogs(launch.id)}>{t("打开日志")}</button>{/if}{/if}</p>{/if}
  <p class="launch-label">{preparing ? t("取消启动") : t("启动游戏")}</p>
</div>
