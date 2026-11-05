<!-- Search leads the catalog. Project names, required mods and compatibility lead the detail view.
     The install panel has one primary action. Reuse the shell's charcoal primary, 8–24px spacing and neutral surfaces. -->
<script module lang="ts">
  export interface ResourceProject {
    project_id: string;
    title: string;
    description: string;
    display_categories: string[];
    downloads: number;
    date_modified: string;
    icon_url: string | null;
    latest_version?: string | null;
  }
  interface ModVersion {
    id: string;
    number: string;
    channel: string;
    published: string;
    filename: string;
    size: number;
    requiredDependencies: number;
    blocked?: boolean;
  }
  interface DependencyDetails {
    version: string;
    items: { name: string | null; version: string | null; iconUrl: string | null; project: ResourceProject | null }[];
  }
  export interface ResourceDetail {
    project: ResourceProject;
    targetId: string;
    versionId: string;
    scrollTop: number;
    options?: { targetKey: string; items: ModVersion[] };
    dependencies?: { versionId: string | null; data: DependencyDetails };
  }
</script>

<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { flip } from 'svelte/animate';
  import { i18n, t } from '../../../ui.shell/frontend/lib/i18n';
  import { action, errorMessage, taskActive, type InstanceInfo, type Snapshot } from '../../../ui.shell/frontend/lib/api';
  import { downloadBytes, downloadItem } from '../../../ui.shell/frontend/lib/downloads';
  import { animateText, entrance, fadeSlide } from '../../../ui.shell/frontend/lib/motion';
  import DownloadProgress from '../../../ui.shell/frontend/components/DownloadProgress.svelte';
  import Icon from '../../../ui.shell/frontend/components/Icon.svelte';

  interface SearchProject extends ResourceProject { categories: string[]; versions: string[] }
  interface SearchPage { hits: SearchProject[]; total_hits: number }
  type Source = 'modrinth' | 'curseforge';
  interface Filters { categories: string[]; gameVersions: string[]; categoryNames?: Record<string, string> }
  interface Query { query: string; gameVersion: string; category: string; loader: string; source: Source }
  let { snapshot, trail = $bindable([]), onchange }: {
    snapshot: Snapshot | null;
    trail: ResourceDetail[];
    onchange: () => Promise<void>;
  } = $props();
  const detail = $derived(trail.at(-1));
  const project = $derived(detail?.project);

  const pageSize = 20;
  const inputDelay = 300;
  const cardMoveDuration = () => matchMedia('(prefers-reduced-motion: reduce)').matches ? 0 : 190;
  const emptyQuery: Query = { query: '', gameVersion: '', category: '', loader: '', source: 'modrinth' };
  const sources = { modrinth: 'Modrinth', curseforge: 'CurseForge' };
  const sourceIds = Object.keys(sources) as Source[];
  const sourceMenuId = $props.id();
  let sourceMenuOpen = $state(false);
  let sourceControl: HTMLDivElement;
  let sourceTrigger: HTMLButtonElement;
  const loaders: Record<string, string> = { fabric: 'Fabric', forge: 'Forge', neoforge: 'NeoForge', quilt: 'Quilt' };
  const categoryLabels: Record<string, string> = {
    adventure: '冒险', cursed: '恶搞', decoration: '装饰', economy: '经济', equipment: '装备', food: '食物',
    'game-mechanics': '游戏机制', library: '支持库', magic: '魔法', management: '管理', minigame: '小游戏',
    mobs: '生物', optimization: '性能优化', social: '社交', storage: '存储', technology: '科技',
    transportation: '运输', utility: '实用', worldgen: '世界生成',
  };
  const categoryName = (key: string) => filters.categoryNames?.[key] ?? t(categoryLabels[key] ?? key);
  function loaderName(instance: InstanceInfo) {
    return loaders[instance.components?.kind || instance.loaderName.toLowerCase()]
      ?? (instance.components && !instance.components.kind ? 'Fabric' : '');
  }

  let query = $state<Query>({ ...emptyQuery });
  let applied = $state<Query>(emptyQuery);
  const browsing = $derived(!applied.query.trim() && !applied.gameVersion.trim() && !applied.category);
  let filters = $state<Filters>({ categories: [], gameVersions: [] });
  const releaseVersions = $derived(new Set(filters.gameVersions));
  let filtersError = $state(false);
  let loadingFilters = $state(false);
  let filterSequence = 0;
  let results = $state<SearchPage>({ hits: [], total_hits: 0 });
  let offset = $state(0);
  let searching = $state(true);
  let searchError = $state('');
  let searchSequence = 0;
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let lastSearch = { criteria: emptyQuery, offset: 0 };
  let list = $state<HTMLElement>();
  let detailPane = $state<HTMLElement>();
  let detailTitle = $state<HTMLHeadingElement>();
  let selectedCard: HTMLButtonElement | undefined;

  const instances = $derived(snapshot?.instances.filter(instance => instance.isInstalled && loaderName(instance)) ?? []);
  let lastTargetId = '';
  const target = $derived(instances.find(instance => instance.id === detail?.targetId)
    ?? instances.find(instance => instance.id === snapshot?.selectedInstanceId) ?? instances[0]);
  const targetKey = $derived(target ? JSON.stringify([snapshot?.dataDirectory, target.id, target.gameVersion, loaderName(target)]) : '');
  const targetBusy = $derived(snapshot?.tasks.some(task => task.instanceId === target?.id && taskActive(task)));
  let versions = $state<ModVersion[]>([]);
  const version = $derived(versions.find(item => item.id === detail?.versionId));
  const showDependencies = $derived(Boolean(version || (project && !target)));
  const dependencyVersionId = $derived(version?.id ?? (!target ? project?.latest_version || null : null));
  let dependencyDetails = $state<DependencyDetails | null>(null);
  let dependencyError = $state('');
  let dependencyRetry = $state(0);
  let loadingVersions = $state(false);
  let versionError = $state('');
  let versionRetry = $state(0);
  let taskId = $state('');
  const task = $derived(snapshot?.tasks.find(item => item.id === taskId));
  let downloadError = $state('');
  let starting = $state(false);

  const counts = $derived(new Intl.NumberFormat($i18n.locale, { notation: 'compact' }));
  const dates = $derived(new Intl.DateTimeFormat($i18n.locale, { dateStyle: 'medium' }));
  const relativeTime = $derived(new Intl.RelativeTimeFormat($i18n.locale, { numeric: 'auto' }));
  function dateLabel(value: string) {
    const timestamp = Date.parse(value);
    return Number.isFinite(timestamp) ? dates.format(timestamp) : '';
  }
  function updated(date: string) {
    const days = Math.max(0, Math.floor((Date.now() - Date.parse(date)) / 86_400_000));
    if (!Number.isFinite(days)) return '';
    if (days >= 365) return relativeTime.format(-Math.floor(days / 365), 'year');
    if (days >= 30) return relativeTime.format(-Math.floor(days / 30), 'month');
    return relativeTime.format(-days, 'day');
  }
  function iconUrl(url: string | null) {
    return url && /^https:\/\/(cdn\.modrinth\.com|(?:media|mediafiles|mediafilez|edge)\.forgecdn\.net)\//.test(url) ? url : undefined;
  }
  function projectCategories(item: ResourceProject) {
    return item.display_categories.filter(category => filters.categories.includes(category) || Object.hasOwn(categoryLabels, category));
  }

  async function loadFilters() {
    const sequence = ++filterSequence;
    loadingFilters = true;
    filtersError = false;
    try { const data = await action<Filters>('resourceFilters', { source: query.source }); if (sequence === filterSequence) filters = data; }
    catch { if (sequence === filterSequence) filtersError = true; }
    finally { if (sequence === filterSequence) loadingFilters = false; }
  }
  function refresh(next: Query, start = 0, delay = 0) {
    clearTimeout(searchTimer);
    const sequence = ++searchSequence;
    const criteria = { ...next };
    lastSearch = { criteria, offset: start };
    searching = true;
    searchError = '';
    if (delay) searchTimer = setTimeout(() => search(criteria, start, sequence), delay);
    else void search(criteria, start, sequence);
  }
  async function search(criteria: Query, start: number, sequence: number) {
    try {
      const result = await action<SearchPage>('resourceSearch', { ...criteria, offset: start });
      if (sequence !== searchSequence) return;
      applied = criteria;
      results = result;
      offset = start;
      list?.scrollTo(0, 0);
    } catch (error) {
      if (sequence === searchSequence) searchError = errorMessage(error);
    } finally {
      if (sequence === searchSequence) searching = false;
    }
  }
  function changeFilters(fields: Partial<Query>, delay = 0) {
    query = { ...query, ...fields };
    if (!query.gameVersion.trim()) query.loader = '';
    refresh(query, 0, delay);
  }
  function reset() {
    changeFilters({ ...emptyQuery, source: query.source });
  }
  function changeSource(source: Source) {
    if (source === query.source) return;
    filters = { categories: [], gameVersions: [] };
    results = { hits: [], total_hits: 0 };
    changeFilters({ source, category: '' });
    void loadFilters();
  }
  async function openSourceMenu(index: number) {
    sourceMenuOpen = true;
    await tick();
    sourceControl?.querySelectorAll<HTMLButtonElement>('.source-option')[index]?.focus();
  }
  function closeSourceMenu() {
    sourceMenuOpen = false;
    sourceTrigger.focus();
  }
  function dismissSourceMenu(event: Event) {
    if (sourceMenuOpen && event.target instanceof Node && !sourceControl.contains(event.target)) {
      sourceMenuOpen = false;
    }
  }
  function sourceKeydown(event: KeyboardEvent, index: number) {
    if (sourceMenuOpen && (event.key === 'Escape' || event.key === 'Tab')) {
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
      }
      closeSourceMenu();
      return;
    }
    const last = sourceIds.length - 1;
    let next: number;
    switch (event.key) {
      case 'ArrowUp': next = index <= 0 ? last : index - 1; break;
      case 'ArrowDown': next = index === last ? 0 : index + 1; break;
      case 'Home': next = 0; break;
      case 'End': next = last; break;
      default: return;
    }
    event.preventDefault();
    void openSourceMenu(next);
  }
  function openProject(item: ResourceProject, trigger: HTMLButtonElement) {
    downloadError = '';
    selectedCard = trigger;
    trail = [{ project: item, targetId: lastTargetId, versionId: '', scrollTop: 0 }];
  }
  function openDependency(item: ResourceProject | null) {
    if (!detail || !item) return;
    const targetId = target?.id ?? '';
    detail.scrollTop = detailPane?.scrollTop ?? 0;
    detail.targetId = targetId;
    trail = [...trail, { project: item, targetId, versionId: '', scrollTop: 0 }];
  }
  function chooseTarget(id: string) {
    if (!detail) return;
    lastTargetId = id;
    detail.targetId = id;
    detail.versionId = '';
  }
  function chooseVersion(id: string) {
    if (detail) detail.versionId = id;
  }
  function applyVersions(entry: ResourceDetail, items: ModVersion[]) {
    versions = items;
    entry.versionId = (items.find(item => item.id === entry.versionId)
      ?? items.find(item => !item.blocked && item.channel === 'release')
      ?? items.find(item => !item.blocked) ?? items[0])?.id ?? '';
  }
  $effect(() => {
    const entry = detail;
    const id = target?.id;
    const key = targetKey;
    void versionRetry;
    versions = [];
    versionError = '';
    downloadError = '';
    loadingVersions = false;
    if (!entry || !id) return;
    const saved = untrack(() => entry.options);
    if (saved?.targetKey === key) {
      untrack(() => applyVersions(entry, saved.items));
      return;
    }
    let current = true;
    loadingVersions = true;
    void action<ModVersion[]>('resourceVersions', { id, project: entry.project.project_id, source: applied.source }).then(items => {
      if (!current) return;
      entry.options = { targetKey: key, items };
      applyVersions(entry, items);
    }).catch(error => {
      if (current) versionError = errorMessage(error);
    }).finally(() => {
      if (current) loadingVersions = false;
    });
    return () => { current = false; };
  });
  $effect(() => {
    const entry = detail;
    const selectedId = dependencyVersionId;
    void dependencyRetry;
    dependencyDetails = null;
    dependencyError = '';
    if (!entry || !showDependencies || version?.requiredDependencies === 0) return;
    const saved = untrack(() => entry.dependencies);
    if (saved?.versionId === selectedId) {
      dependencyDetails = saved.data;
      return;
    }
    let current = true;
    void action<DependencyDetails>('resourceDependencies', { project: entry.project.project_id, versionId: selectedId, source: applied.source }).then(data => {
      if (!current) return;
      entry.dependencies = { versionId: selectedId, data };
      dependencyDetails = data;
    }).catch(error => {
      if (current) dependencyError = errorMessage(error);
    });
    return () => { current = false; };
  });
  $effect(() => {
    const entry = detail;
    if (entry) {
      void tick().then(() => {
        if (entry !== detail) return;
        detailPane?.scrollTo(0, entry.scrollTop);
        detailTitle?.focus({ preventScroll: true });
      });
    } else if (selectedCard) {
      const card = selectedCard;
      selectedCard = undefined;
      void tick().then(() => {
        if (card.isConnected && !card.closest('[inert]')) card.focus({ preventScroll: true });
      });
    }
  });
  async function download() {
    if (!target || !project || !version || version.blocked || starting || targetBusy || snapshot?.readOnly) return;
    starting = true;
    downloadError = '';
    try {
      taskId = await action<string>('resourceDownload', { id: target.id, project: project.project_id, versionId: version.id, source: applied.source });
      await onchange();
    } catch (error) { downloadError = errorMessage(error); }
    finally { starting = false; }
  }
  async function cancel() {
    try { await action('cancelTask', { id: taskId }); await onchange(); }
    catch (error) { downloadError = errorMessage(error); }
  }
  onMount(() => {
    void loadFilters();
    refresh(emptyQuery);
    return () => {
      clearTimeout(searchTimer);
      searchSequence++;
    };
  });
</script>

<svelte:window onpointerdown={dismissSourceMenu} onfocusin={dismissSourceMenu} />

{#snippet projectIcon(url: string | null, placeholderSize = 24)}
  {@const src = iconUrl(url)}
  <span class="project-icon">
    <Icon name="puzzle" size={placeholderSize} />
    {#if src}
      {#key src}
        <img {src} alt="" referrerpolicy="no-referrer" loading="lazy" onload={event => event.currentTarget.setAttribute('data-loaded', '')} />
      {/key}
    {/if}
  </span>
{/snippet}

<div class="resources-page">
  <div class="catalog" hidden={!!project}>
    <form class="search-form" role="search" aria-labelledby="resource-search-title" onsubmit={event => { event.preventDefault(); refresh(query); }}>
      <div class="catalog-hero">
        <div class="hero-art" aria-hidden="true"><Icon name="cube" size={168} width={0.65} /></div>
        <div class="hero-heading">
          <div>
            <h2 id="resource-search-title">{t("发现模组")}</h2>
            <p>{t("为你的 Minecraft 实例查找模组。")}</p>
          </div>
          <div class="source-badge">
            <span>{t("当前来源")}</span>
            <div class="source-control" bind:this={sourceControl}>
              <button type="button" class="source-trigger" bind:this={sourceTrigger} aria-haspopup="menu"
                      aria-expanded={sourceMenuOpen} aria-controls={sourceMenuOpen ? sourceMenuId : undefined}
                      aria-label={`${t("当前来源")} ${sources[query.source]}`}
                      onclick={() => sourceMenuOpen ? closeSourceMenu() : void openSourceMenu(sourceIds.indexOf(query.source))}
                      onkeydown={event => sourceKeydown(event, -1)}>
                <Icon name={query.source} size={16} />
                {sources[query.source]}
                <span class="source-arrow"><Icon name="chevron" size={14} /></span>
              </button>
              {#if sourceMenuOpen}
                <div id={sourceMenuId} class="source-menu" role="menu" aria-label={t("当前来源")}>
                  {#each sourceIds as source, index (source)}
                    <button type="button" class="source-option" role="menuitemradio" aria-checked={query.source === source} tabindex="-1"
                            onclick={() => { changeSource(source); closeSourceMenu(); }} onkeydown={event => sourceKeydown(event, index)}>
                      <Icon name={source} size={16} />
                      {sources[source]}
                      {#if query.source === source}<span class="source-check"><Icon name="check" size={14} /></span>{/if}
                    </button>
                  {/each}
                </div>
              {/if}
            </div>
          </div>
        </div>
        <div class="search-bar">
          <div class="search-input">
            <Icon name="search" size={18} />
            <input type="search" value={query.query} maxlength="160" aria-label={t("搜索模组名称")} placeholder={t("按名称搜索，例如 Sodium")} oninput={event => {
              if (!(event instanceof InputEvent && event.isComposing)) changeFilters({ query: event.currentTarget.value }, inputDelay);
            }} />
          </div>
          <button type="submit" class="primary search-submit">{t("搜索")}</button>
        </div>
      </div>
      <div class="filter-bar">
        <label class="version-filter">
          <span>{t("游戏版本")}</span>
          <input value={query.gameVersion} maxlength="40" list="resource-game-versions" placeholder={t("全部（可手动输入）")} oninput={event => changeFilters({ gameVersion: event.currentTarget.value }, inputDelay)} />
        </label>
        <datalist id="resource-game-versions">{#each filters.gameVersions as gameVersion (gameVersion)}<option value={gameVersion}></option>{/each}</datalist>
        {#if query.gameVersion.trim()}
          <label class="loader-filter" transition:fadeSlide={{ compact: true }}>
            <span>{t("加载器")}</span>
            <select value={query.loader} onchange={event => changeFilters({ loader: event.currentTarget.value })}>
              <option value="">{t("全部")}</option>
              {#each Object.entries(loaders) as [id, name] (id)}<option value={id}>{name}</option>{/each}
            </select>
          </label>
        {/if}
        <label><span>{t("类型")}</span><select value={query.category} onchange={event => changeFilters({ category: event.currentTarget.value })}><option value="">{t("全部")}</option>{#each filters.categories as category (category)}<option value={category}>{categoryName(category)}</option>{/each}</select></label>
        <button type="button" class="text-button reset-filters" onclick={reset}>{t("重置条件")}</button>
        {#if filtersError}<button type="button" class="text-button" disabled={loadingFilters} onclick={loadFilters}>{t("重试加载筛选项")}</button>{/if}
      </div>
    </form>

    <div class="results-bar">
      <div class="results-heading">
        <h3>{browsing ? t("热门模组") : t("搜索结果")}</h3>
        <p class="muted result-feedback" role="status" use:animateText={searching ? 'searching' : results.total_hits}>
          {#if searching}
            <span class="spinner"><Icon name="spinner" size={14} /></span>
            {results.hits.length ? t("正在更新结果…") : t("正在搜索模组…")}
          {:else if !searchError}
            {t("{0} 个项目", results.total_hits.toLocaleString($i18n.locale))}
          {/if}
        </p>
      </div>
      {#if !searchError && results.hits.length}
        <nav class="pagination" aria-label={t("结果分页")}>
          <button type="button" class="icon-button" aria-label={t("上一页")} title={t("上一页")} disabled={searching || offset === 0} onclick={() => refresh(applied, offset - pageSize)}><Icon name="back" size={16} /></button>
          <span class="muted">{t("第 {0} 页", offset / pageSize + 1)}</span>
          <button type="button" class="icon-button" aria-label={t("下一页")} title={t("下一页")} disabled={searching || offset + pageSize >= results.total_hits || offset + pageSize > 10_000} onclick={() => refresh(applied, offset + pageSize)}><Icon name="chevron" size={16} /></button>
        </nav>
      {/if}
    </div>

    <section class="results scroll-pane" bind:this={list} aria-label={t("模组搜索结果")} aria-busy={searching}>
      {#if searchError}
        <div class="catalog-state" in:entrance>
          <p class="issue" role="alert">{searchError}</p>
          <button type="button" onclick={() => refresh(lastSearch.criteria, lastSearch.offset)}>{t("重试")}</button>
        </div>
      {:else if !results.hits.length}
        {#if searching}
          <div class="project-grid" aria-hidden="true">
            {#each [0, 1, 2, 3] as index}
              <div class="skeleton-card" style:--skeleton-delay={index * 70 + 'ms'}><span></span><span></span><span></span></div>
            {/each}
          </div>
        {:else}
          <div class="catalog-state" role="status" in:entrance><Icon name="search" size={24} /><p>{t("没有符合条件的模组。")}</p></div>
        {/if}
      {:else}
        <div class="project-grid">
          {#each results.hits as item, index (item.project_id)}
            {@const platforms = applied.loader ? [loaders[applied.loader]] : item.categories.filter(category => Object.hasOwn(loaders, category)).map(category => loaders[category])}
            {@const gameVersion = applied.gameVersion.trim() || item.versions.filter(value => releaseVersions.has(value)).at(-1) || item.versions.at(-1)}
            <button type="button" class="project-card" aria-labelledby={'resource-project-' + item.project_id} onclick={event => openProject(item, event.currentTarget)} animate:flip={{ duration: cardMoveDuration }} in:entrance|global={{ delay: Math.min(index, 5) * 20 }}>
              <span class="card-heading">
                {@render projectIcon(item.icon_url)}
                <span class="card-identity">
                  <strong id={'resource-project-' + item.project_id} class="project-title truncate" title={item.title}>{item.title}</strong>
                  <span class="tags">
                    {#each projectCategories(item).slice(0, 2) as category (category)}
                      <span class="tag">{categoryName(category)}</span>
                    {/each}
                  </span>
                </span>
                <span class="card-arrow"><Icon name="chevron" size={16} /></span>
              </span>
              <span class="project-summary" title={item.description}>{item.description}</span>
              <span class="compatibility" title={[...(applied.loader ? [gameVersion] : item.versions), ...platforms].join(' / ')} use:animateText={gameVersion + '|' + applied.loader}>
                <Icon name="cube" size={14} />{gameVersion ?? ''}
                {#if platforms.length}<span class="truncate">· {platforms.join(' / ')}</span>{/if}
              </span>
              <span class="card-footer">
                <span title={t("{0} 次下载", item.downloads.toLocaleString($i18n.locale))}><Icon name="download" size={14} />{counts.format(item.downloads)}</span>
                <span title={t("更新时间") + ' · ' + dateLabel(item.date_modified)}><Icon name="clock" size={14} />{updated(item.date_modified)}</span>
                <span class="card-source">{sources[applied.source]}</span>
              </span>
            </button>
          {/each}
        </div>
      {/if}
    </section>
  </div>

  {#if project}
    {#key detail}
    <section class="details scroll-pane" aria-label={project.title} bind:this={detailPane}>
      <div class="detail-main">
        <div class="detail-overview" in:fadeSlide|global={{ compact: true }}>
          <div class="detail-heading">
            {@render projectIcon(project.icon_url)}
            <div><p class="muted">{sources[applied.source]}</p><h2 bind:this={detailTitle} tabindex="-1">{project.title}</h2></div>
          </div>
          <p class="description">{project.description}</p>
          <div class="tags">
            {#each projectCategories(project) as category (category)}<span class="tag">{categoryName(category)}</span>{/each}
          </div>
          <div class="overview-meta">
            <span><Icon name="download" size={16} />{t("{0} 次下载", project.downloads.toLocaleString($i18n.locale))}</span>
            <span title={dateLabel(project.date_modified)}><Icon name="clock" size={16} />{t("更新时间")} · {updated(project.date_modified)}</span>
          </div>
        </div>
        {#if showDependencies}
          <section class="required-mods" aria-labelledby="resource-dependency-title" in:entrance>
            <div class="dependency-heading">
              <h3 id="resource-dependency-title">{t("必需前置模组")}</h3>
              {#if !version}<span class="muted">{t("{0} 最新版本", sources[applied.source])}{dependencyDetails ? ' · ' + dependencyDetails.version : ''}</span>{/if}
            </div>
            {#if dependencyError}
              <p class="issue" role="alert">{dependencyError}</p>
              <button type="button" class="text-button" onclick={() => dependencyRetry++}>{t("重试")}</button>
            {:else if version?.requiredDependencies === 0 || dependencyDetails?.items.length === 0}
              <p class="muted">{t("此版本无需前置模组。")}</p>
            {:else if dependencyDetails}
              <ul class="dependency-list" in:entrance>
                {#each dependencyDetails.items as dependency}
                  <li>
                    <button type="button" class="dependency-row" disabled={!dependency.project} onclick={() => openDependency(dependency.project)}>
                      {@render projectIcon(dependency.iconUrl, 16)}
                      <span class="dependency-name">
                        <strong>{dependency.name ?? t("未知前置模组")}</strong>
                        {#if dependency.version}<small>{t("指定版本 {0}", dependency.version)}</small>{/if}
                        {#if !dependency.project}<small>{t("暂无项目详情")}</small>{/if}
                      </span>
                      {#if dependency.project}<span class="dependency-arrow"><Icon name="chevron" size={16} /></span>{/if}
                    </button>
                  </li>
                {/each}
              </ul>
              {#if version}<p class="muted">{t("下载时会自动补齐缺失的前置模组。")}</p>{/if}
            {:else}
              <p class="operation-status dependency-loading" role="status"><span class="spinner"><Icon name="spinner" size={15} /></span>{t("正在读取前置模组…")}</p>
            {/if}
          </section>
        {/if}
      </div>
      <aside class="install-panel" aria-labelledby="resource-install-title" in:entrance|global={{ delay: 40 }}>
        <h3 id="resource-install-title"><Icon name="download" size={19} />{t("安装配置")}</h3>
        {#if target}
          <label class="detail-field">
            <span>{t("目标实例")}</span>
            <select value={target.id} onchange={event => chooseTarget(event.currentTarget.value)}>
              {#each instances as instance (instance.id)}
                <option value={instance.id}>{instance.name} · {instance.gameVersion} · {loaderName(instance)}</option>
              {/each}
            </select>
          </label>
          {#if loadingVersions}
            <p class="muted" role="status">{t("正在获取兼容版本…")}</p>
          {:else if versionError}
            <p class="issue" role="alert">{versionError}</p>
            <button type="button" class="text-button" onclick={() => versionRetry++}>{t("重试")}</button>
          {:else if version}
            <label class="detail-field">
              <span>{t("模组版本")}</span>
              <select value={version.id} onchange={event => chooseVersion(event.currentTarget.value)}>
                {#each versions as item (item.id)}
                  <option value={item.id}>{item.number} · {item.channel === 'release' ? t("正式版") : item.channel}</option>
                {/each}
              </select>
            </label>
            <div class="install-file" use:animateText={version.id}>
              <p class="filename">{version.filename}</p>
              <p class="muted">{downloadBytes(version.size)} · {dateLabel(version.published)}</p>
            </div>
            {#if targetBusy && !task}<p class="muted">{t("目标实例正在运行游戏或处理任务。")}</p>{/if}
            {#if snapshot?.readOnly}<p class="muted">{t("启动器当前处于只读模式。")}</p>{/if}
            {#if version.blocked}<p class="muted">{t("此文件未开放第三方下载。")}</p>{/if}
            <button type="button" class="primary full-width" disabled={starting || targetBusy || snapshot?.readOnly || version.blocked} onclick={download}>{starting ? t("正在提交…") : t("下载到实例")}</button>
          {:else}
            <p class="muted" role="status">{t("此模组没有与目标实例兼容的版本。")}</p>
          {/if}
        {:else}
          <p class="muted" role="status">{t("请先在版本选择中安装带模组加载器的游戏实例。")}</p>
        {/if}
      </aside>
    </section>
    {/key}
  {/if}

  {#if task || downloadError}
    <div class="task-status" in:entrance>
      {#if task}
        {@const progress = downloadItem(task)}
        <div class="task-line">
          <p class:issue={task.phase === 'failed'} class="muted" role="status" use:animateText={task.phase + task.message}>
            {task.instanceName} · {task.phase === 'completed' ? t("模组已安装") : t(task.message)}
          </p>
          {#if taskActive(task) && task.phase !== 'committing'}<button class="text-button" onclick={cancel}>{t("取消")}</button>{/if}
        </div>
        {#if taskActive(task)}<DownloadProgress percent={progress.percent} indeterminate={progress.indeterminate} />{/if}
      {/if}
      {#if downloadError}<p class="issue" role="alert">{downloadError}</p>{/if}
    </div>
  {/if}
</div>

<style>
  .resources-page { display: flex; flex-direction: column; flex: 1; min-height: 0; gap: 12px; padding: 12px 24px 20px; }
  .catalog { display: flex; flex-direction: column; flex: 1; min-height: 0; }
  .catalog[hidden] { display: none; }
  .search-form { flex: none; }
  .catalog-hero {
    position: relative;
    display: grid;
    gap: 12px;
    padding: 16px 20px;
    border-radius: 12px;
    background: #f2f3f6;
  }
  .hero-art { position: absolute; inset: 0; overflow: hidden; border-radius: inherit; color: var(--secondary); opacity: .1; pointer-events: none; }
  .hero-art > :global(svg) { position: absolute; right: 52px; top: -31px; transform: rotate(-12deg); }
  .hero-heading { position: relative; display: flex; justify-content: space-between; align-items: flex-start; gap: 12px; }
  .hero-heading h2 { font-size: 26px; font-weight: 600; line-height: 1.15; }
  .hero-heading p { margin-top: 5px; color: var(--secondary); font-size: 13px; line-height: 1.45; }
  .source-badge { display: inline-flex; align-items: center; gap: 8px; font-size: 12px; white-space: nowrap; }
  .source-badge > span:first-child { color: var(--secondary); }
  .source-control { position: relative; }
  .source-trigger, .source-option { display: flex; align-items: center; gap: 8px; width: 100%; min-height: 32px; padding: 4px 8px; font-size: 13px; box-shadow: none; }
  .source-trigger { min-width: 144px; border-radius: 6px; }
  .source-arrow { margin-left: auto; transform: rotate(90deg); }
  .source-menu { position: absolute; z-index: 2; inset: calc(100% + 4px) 0 auto; display: grid; gap: 2px; padding: 4px; border: 1px solid var(--stroke); border-radius: 8px; background: #fff; box-shadow: 0 4px 12px #18202b20; }
  .source-option { border: 0; border-radius: 4px; text-align: left; }
  .source-option[aria-checked='true'] { background: #eef0f3; }
  .source-option:focus-visible { outline-offset: -2px; }
  .source-check { margin-left: auto; }
  .search-bar { position: relative; display: flex; gap: 8px; }
  .search-input {
    display: flex;
    align-items: center;
    flex: 1;
    min-width: 0;
    gap: 8px;
    padding: 0 10px;
    border: 1px solid var(--stroke);
    border-radius: 8px;
    background: #f6f7f9;
    color: var(--secondary);
  }
  .search-input:focus-within { outline: 2px solid #34373c; outline-offset: 2px; }
  .search-input input { min-width: 0; border: 0; padding: 6px 0; outline: none; background: transparent; color: #17191c; }
  .search-submit { padding-inline: 20px; box-shadow: none; font-size: 14px; font-weight: 600; white-space: nowrap; }
  .filter-bar { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 12px; margin-top: 10px; }
  .filter-bar label { display: flex; align-items: center; gap: 6px; color: var(--secondary); font-size: 13px; white-space: nowrap; }
  .filter-bar input, .filter-bar select { width: 144px; min-height: 34px; padding-block: 5px; color: #17191c; font-size: 13px; }
  .filter-bar .version-filter input { width: 168px; }
  .filter-bar .loader-filter select { width: 116px; }
  .reset-filters { margin-left: auto; }

  .results-bar { display: flex; justify-content: space-between; align-items: center; gap: 8px; min-height: 32px; margin: 12px 0 6px; }
  .results-heading, .pagination { display: flex; align-items: center; gap: 8px; }
  .results-heading h3 { font-size: 16px; font-weight: 600; }
  .result-feedback { display: flex; align-items: center; gap: 5px; }
  .pagination { gap: 4px; white-space: nowrap; }
  .pagination button { width: 28px; height: 28px; }
  .pagination button:disabled { opacity: .55; }
  .results { flex: 1; padding: 2px 4px 4px 0; }
  .catalog-state {
    height: 100%;
    min-height: 120px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    gap: 8px;
    padding: 16px;
    border-radius: 8px;
    background: #f6f7f9;
    color: var(--secondary);
    text-align: center;
    font-size: 14px;
  }
  .project-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 264px), 1fr)); gap: 10px; }
  .project-card {
    display: grid;
    min-width: 0;
    gap: 6px;
    padding: 12px;
    border: 1px solid var(--stroke);
    border-radius: 10px;
    box-shadow: none;
    text-align: left;
  }
  .project-card:hover:enabled { border-color: #8c98ac; background: #f6f7f9; }
  .project-card:focus-visible { outline-offset: -2px; }
  .skeleton-card { min-height: 158px; display: grid; align-content: start; gap: 12px; padding: 12px; border: 1px solid var(--stroke); border-radius: 10px; }
  .skeleton-card span { height: 10px; width: 88%; border-radius: 4px; background: #eef0f3; }
  .skeleton-card span:first-child { width: 42px; height: 42px; border-radius: 8px; }
  .skeleton-card span:last-child { width: 62%; }
  .card-heading { display: flex; align-items: center; gap: 10px; min-width: 0; }
  .card-identity { flex: 1; min-width: 0; display: grid; gap: 3px; }
  .card-arrow { display: grid; place-items: center; width: 26px; height: 26px; border-radius: 6px; background: #f2f3f6; color: var(--secondary); }
  .project-icon { position: relative; flex: 0 0 42px; width: 42px; height: 42px; display: grid; place-items: center; border-radius: 8px; overflow: hidden; background: #e8ecf3; color: var(--secondary); }
  .project-icon img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; background: #f6f7f9; }
  .project-icon img:not([data-loaded]) { opacity: 0; }
  .truncate { min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  .project-title { font-size: 15px; font-weight: 600; }
  .tags { display: flex; flex-wrap: wrap; gap: 4px; }
  .tag { padding: 1px 6px; border-radius: 4px; background: #eef0f3; color: var(--secondary); font-size: 11px; line-height: 1.5; }
  .card-identity .tags { flex-wrap: nowrap; overflow: hidden; white-space: nowrap; }
  .project-summary {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    height: 34px;
    overflow: hidden;
    overflow-wrap: anywhere;
    color: var(--secondary);
    font-size: 13px;
    line-height: 17px;
  }
  .compatibility, .card-footer, .card-footer > span { display: flex; align-items: center; gap: 4px; color: var(--secondary); font-size: 12px; white-space: nowrap; }
  .compatibility { min-width: 0; }
  .card-footer { gap: 10px; border-top: 1px solid #e2e5eb; padding-top: 6px; }
  .card-footer .card-source { margin-left: auto; font-size: 11px; }

  .details {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 280px), 1fr));
    align-content: start;
    align-items: start;
    gap: 16px;
    padding: 2px 4px 8px 0;
    overflow-wrap: anywhere;
  }
  .detail-main { display: grid; min-width: 0; gap: 12px; }
  .detail-overview, .required-mods, .install-panel { display: grid; min-width: 0; gap: 12px; padding: 16px; border: 1px solid var(--stroke); border-radius: 10px; }
  .detail-heading { display: flex; align-items: center; gap: 12px; }
  .detail-heading h2 { font-size: 24px; font-weight: 600; line-height: 1.2; }
  .detail-heading h2:focus { outline: none; }
  .detail-heading .project-icon { flex-basis: 64px; width: 64px; height: 64px; }
  .description { font-size: 14px; color: var(--secondary); line-height: 1.65; }
  .detail-overview .tags:empty { display: none; }
  .overview-meta { display: grid; gap: 8px; border-top: 1px solid #e2e5eb; padding-top: 12px; color: var(--secondary); font-size: 13px; }
  .overview-meta > span, .install-panel h3 { display: flex; align-items: center; gap: 8px; }
  .dependency-heading { display: flex; align-items: baseline; justify-content: space-between; flex-wrap: wrap; gap: 4px 12px; }
  .dependency-heading h3, .install-panel h3 { font-size: 16px; font-weight: 600; }
  .dependency-list { display: grid; gap: 8px; margin: 0; padding: 0; list-style: none; }
  .dependency-row { display: flex; align-items: center; gap: 8px; width: 100%; padding: 8px 10px; border: 0; border-radius: 6px; box-shadow: none; background: #f6f7f9; font-size: 14px; text-align: left; }
  .dependency-row:hover:enabled { background: #eef0f3; }
  .dependency-row:disabled { color: #17191c; }
  .dependency-name { flex: 1; min-width: 0; }
  .dependency-arrow { color: var(--secondary); }
  .dependency-list .project-icon { flex-basis: 28px; width: 28px; height: 28px; border-radius: 6px; }
  .dependency-list small { display: block; margin-top: 2px; color: var(--secondary); font-size: 12px; }
  .dependency-loading { margin: 0; }
  .required-mods .text-button { justify-self: start; }
  .install-panel { background: #f6f7f9; }
  .detail-field { display: grid; min-width: 0; gap: 6px; font-size: 13px; }
  .detail-field select { min-width: 0; overflow: hidden; text-overflow: ellipsis; font-size: 14px; }
  .install-file { padding: 8px 10px; border-radius: 6px; background: #eef0f3; }
  .filename { font-size: 13px; }
  .task-status { padding: 8px 12px; border: 1px solid var(--stroke); border-radius: 8px; background: #fff; }
  .task-line { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  @media (prefers-reduced-motion: no-preference) {
    .catalog:not([hidden]) { animation: catalog-in 200ms ease-out both; }
    .skeleton-card span { animation: skeleton-pulse 850ms ease-in-out var(--skeleton-delay) infinite alternate; }
    .project-card { transition: translate 160ms ease-out, box-shadow 160ms ease-out, background 160ms ease-out, border-color 160ms ease-out; }
    .project-card:hover:enabled { translate: 0 -2px; box-shadow: 0 4px 8px #18202b12; }
    .project-card:active:enabled { translate: 0 1px; }
    .card-arrow { transition: translate 160ms ease-out; }
    .project-card:hover .card-arrow { translate: 2px 0; }
    .dependency-arrow { transition: translate 160ms ease-out; }
    .dependency-row:hover .dependency-arrow { translate: 2px 0; }
    .project-icon img { transition: opacity 160ms ease-out; }
    @keyframes catalog-in { from { opacity: .6; translate: 0 6px; } to { opacity: 1; translate: 0 0; } }
    @keyframes skeleton-pulse { from { opacity: .48; } to { opacity: 1; } }
  }
</style>
