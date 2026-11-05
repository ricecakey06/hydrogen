<script lang="ts">
  import { action, errorMessage } from '../../../ui.shell/frontend/lib/api';
  import { t } from '../../../ui.shell/frontend/lib/i18n';
  import Icon from '../../../ui.shell/frontend/components/Icon.svelte';
  import launcherLicense from '../../../../LICENSE?raw';
  import miSansLicense from '../../../../licenses/MiSans.txt?raw';
  import launcherIcon from '../../../../src-tauri/icons/icon.png';
  import developerAvatar from './developer-avatar.jpg';
  import bangbang93Avatar from './bangbang93-avatar.webp';

  type Update = { status: 'available'; version: string } | { status: 'upToDate' | 'noRelease' };
  type Notice = { name: string; version: string; ecosystem: string; license: string; texts: number[] };
  type Notices = { packages: Notice[]; texts: string[] };

  const links = {
    developer: 'https://space.bilibili.com/365980549',
    bmclapi: 'https://www.bangbang93.com/',
    releases: 'https://github.com/ricecakey06/hydrogen/releases/latest',
    miSansLicense: 'https://hyperos.mi.com/font-download/MiSans字体知识产权许可协议.pdf',
  };

  let { version }: { version?: string } = $props();
  let checking = $state(false);
  let update = $state<Update>();
  let updateError = $state('');
  let linkError = $state('');
  let notices = $state.raw<Notices>();
  let loading = $state(false);
  let licenseError = $state('');
  let query = $state('');
  let selected = $state.raw<Notice>();
  const matches = $derived(notices?.packages.filter(({ name, version, ecosystem, license }) =>
    `${name} ${version} ${ecosystem} ${license}`.toLowerCase().includes(query.trim().toLowerCase())) ?? []);
  const displayed = $derived(selected && matches.includes(selected) ? selected : matches[0]);

  async function checkUpdate() {
    if (checking) return;
    checking = true;
    update = undefined;
    updateError = '';
    try { update = await action<Update>('checkLauncherUpdate'); }
    catch (error) { updateError = errorMessage(error); }
    finally { checking = false; }
  }

  async function openLink(event: MouseEvent, link: keyof typeof links) {
    event.preventDefault();
    linkError = '';
    try { await action('openAboutLink', { link }); }
    catch (error) { linkError = errorMessage(error); }
  }

  async function loadNotices() {
    if (loading || notices) return;
    loading = true;
    licenseError = '';
    try { notices = JSON.parse(await action<string>('aboutLicenses')) as Notices; }
    catch (error) { licenseError = errorMessage(error); }
    finally { loading = false; }
  }
</script>

<!-- Primary: version and update check. Secondary: developer and credits. License text uses the existing form spacing and muted captions. -->
<div class="about-page scroll-pane form-scroll">
  <section class="about-overview" aria-labelledby="about-product">
    <div class="field-row about-version">
      <div class="about-product">
        <img src={launcherIcon} width="56" height="56" alt="" />
        <div>
          <h2 id="about-product">Hydrogen</h2>
          {#if version}<p class="caption">{t("启动器版本 {0}", version)}</p>{/if}
        </div>
      </div>
      <button type="button" class="primary" disabled={checking || !version} onclick={checkUpdate}>
        <Icon name="refresh" size={16} />{checking ? t("正在检查更新…") : t("检查更新")}
      </button>
    </div>
    <div role="status" class="caption">
      {#if updateError}<span class="issue">{t("检查更新失败：{0}", updateError)}</span>
      {:else if update?.status === 'available'}
        {t("发现新版本 {0}。", update.version)}
        <a href={links.releases} onclick={event => openLink(event, 'releases')}>{t("查看发布页")}</a>
      {:else if update?.status === 'upToDate'}{t("已是最新版本。")}
      {:else if update?.status === 'noRelease'}{t("暂未找到 Tauri 版的正式发布。")}{/if}
    </div>
  </section>

  <section aria-labelledby="about-developer">
    <h2 id="about-developer" class="section-label">{t("开发者")}</h2>
    <div class="about-person">
      <img src={developerAvatar} width="48" height="48" alt="" />
      <div><strong>{t("米玖澪")}</strong><p class="caption"><a href={links.developer} onclick={event => openLink(event, 'developer')}>{t("哔哩哔哩主页")}</a></p></div>
    </div>
  </section>

  <section aria-labelledby="about-credits">
    <h2 id="about-credits" class="section-label">{t("鸣谢")}</h2>
    <div class="about-person">
      <img src={bangbang93Avatar} width="48" height="48" alt="" />
      <div>
        <strong>bangbang93</strong>
        <p class="caption">{t("BMCLAPI 开发者 · 为启动器提供下载镜像")}</p>
        <a class="caption" href={links.bmclapi} onclick={event => openLink(event, 'bmclapi')}>{t("访问网站")}</a>
      </div>
    </div>
  </section>
  {#if linkError}<p class="issue" role="alert">{linkError}</p>{/if}

  <section aria-labelledby="about-licenses">
    <h2 id="about-licenses" class="section-label">{t("许可与版权声明")}</h2>
    <details>
      <summary>{t("Hydrogen 许可证（MIT）")}</summary>
      <pre>{launcherLicense.trim()}</pre>
    </details>
    <details>
      <summary>{t("MiSans 字体知识产权许可协议")}</summary>
      <p class="caption">{t("启动器内置 MiSans 字体，授权方为小米科技有限责任公司。")}</p>
      <p class="caption" lang="en">Copyright © 2020-2025 Beijing Xiaomi Mobile Software Co.,Ltd. All Rights Reserved.</p>
      <a class="caption" href={links.miSansLicense} onclick={event => openLink(event, 'miSansLicense')}>{t("查看小米官方协议 PDF")}</a>
      <pre>{miSansLicense}</pre>
    </details>
    <details ontoggle={event => { if (event.currentTarget.open) void loadNotices(); }}>
      <summary>{t("第三方许可证与版权声明")}</summary>
      {#if loading}<p class="caption" role="status">{t("正在读取许可证…")}</p>
      {:else if licenseError}
        <p class="issue" role="alert">{licenseError}</p>
        <button type="button" onclick={loadNotices}>{t("重试")}</button>
      {:else if notices}
        <p class="caption">{t("共 {0} 项，包含前端、原生启动器及官方插件的依赖。", notices.packages.length)}</p>
        <label class="caption" for="about-license-search">{t("按名称、版本或许可证筛选")}</label>
        <input id="about-license-search" type="search" bind:value={query} />
        {#if displayed}
          <label class="caption" for="about-license-package">{t("依赖项")}</label>
          <select id="about-license-package" value={displayed} onchange={event => selected = matches[event.currentTarget.selectedIndex]}>
            {#each matches as item (`${item.ecosystem}/${item.name}/${item.version}`)}
              <option value={item}>{item.name} {item.version} · {item.ecosystem}</option>
            {/each}
          </select>
          <p class="caption">{displayed.license}</p>
          {#each displayed.texts as text}<pre>{notices.texts[text]}</pre>{/each}
        {:else}<p class="caption" role="status">{t("没有匹配的依赖项。")}</p>{/if}
      {/if}
    </details>
  </section>
</div>

<style>
  .about-page { padding-bottom: 24px; }
  section + section { margin-top: 20px; }
  .about-overview, .about-person, details { background: #f2f3f6; border-radius: 8px; padding: 12px 16px; }
  .about-overview h2 { font-size: 24px; font-weight: 600; }
  .about-overview .caption:empty { display: none; }
  .about-version { flex-wrap: wrap; }
  .primary, .about-person, .about-product { display: flex; align-items: center; gap: 12px; }
  .about-product img { flex: none; object-fit: contain; }
  .about-person img { flex: none; border-radius: 50%; object-fit: cover; }
  .about-person strong { font-weight: 600; }
  .about-person .caption { margin-top: 2px; }
  a { color: inherit; text-underline-offset: 2px; }
  a:focus-visible { outline: 2px solid currentColor; outline-offset: 3px; }
  details + details { margin-top: 8px; }
  summary { cursor: pointer; }
  label { display: block; margin: 12px 0 4px; }
  pre { margin: 8px 0 0; max-inline-size: 75ch; white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; font-size: 13px; line-height: 1.6; }
</style>
