<script lang="ts">
  import { onDestroy } from 'svelte'
  import { save } from '@tauri-apps/plugin-dialog'
  import { writeFile } from '@tauri-apps/plugin-fs'
  import { getStore } from '$lib/db'
  import { locale, t, type I18nKey, type I18nParams } from '$lib/i18n'
  import { LatestRequestGuard } from '$lib/item-view-load-guards'
  import {
    buildFrequenciesAsync,
    topN,
    computeTicks,
    truncateLabel,
    type CorpusText,
    type WordFrequency,
  } from '$lib/text-analysis'
  import { DEFAULT_STOPWORDS } from '$lib/stopwords'
  import { layoutWordCloud, CLOUD_FONT_STACK, CLOUD_FONT_WEIGHT } from '$lib/cloud-layout'
  import {
    clampCloudTermCount,
    defaultAnalysisSettings,
    loadAnalysisSettings,
    parseStopwordsInput,
    saveAnalysisSettings,
    MAX_CLOUD_TERMS,
    MIN_CLOUD_TERMS,
    type CollectionAnalysisSettings,
  } from '$lib/analysis-settings'
  import { Panel, Button, TabList, TabButton, IconButton, ActionIcon } from '@entropia/ui'

  let { collectionId, refreshToken }: { collectionId: string; refreshToken: number } = $props()

  const currentLocale = locale
  const translate = $derived.by(() => {
    $currentLocale
    return (key: I18nKey, params?: I18nParams) => t(key, params)
  })

  let status = $state<'loading' | 'ready' | 'error'>('loading')
  let corpus = $state<CorpusText[] | null>(null)
  let frequencies = $state<WordFrequency[]>([])
  let settings = $state<CollectionAnalysisSettings>(defaultAnalysisSettings())
  let stopwordsInput = $state('')
  let activeTab = $state<'viz' | 'settings'>('viz')

  const fetchGuard = new LatestRequestGuard()
  const computeGuard = new LatestRequestGuard()
  let activeSettingsId: string | null = null
  let stopwordsTimer: ReturnType<typeof setTimeout> | undefined
  let wordCloudSvgEl: SVGSVGElement | undefined = $state()
  let barChartSvgEl: SVGSVGElement | undefined = $state()

  let cloudWords = $derived(topN(frequencies, settings.cloudTermCount))
  let top20 = $derived(topN(frequencies, 20))
  let totalTokens = $derived(frequencies.reduce((sum, f) => sum + f.count, 0))

  // Word cloud geometry (SVG viewBox units)
  const CLOUD_W = 480
  const CLOUD_H = 320
  const PNG_EXPORT_SCALE = 3

  let placedWords = $derived(layoutWordCloud(cloudWords, { width: CLOUD_W, height: CLOUD_H }))

  // Bar chart geometry (SVG viewBox units)
  const CHART_W = 480
  const CHART_H = 300
  const MARGIN = { top: 8, right: 8, bottom: 72, left: 36 }
  const INNER_W = CHART_W - MARGIN.left - MARGIN.right
  const INNER_H = CHART_H - MARGIN.top - MARGIN.bottom
  const SLOT_W = INNER_W / 20
  const BAR_W = SLOT_W * 0.68

  let ticks = $derived(computeTicks(top20.length > 0 ? top20[0]!.count : 0, 4))
  let chartMax = $derived(Math.max(ticks[ticks.length - 1] ?? 1, 1))

  function barHeight(count: number): number {
    return (count / chartMax) * INNER_H
  }

  function tickY(tick: number): number {
    return MARGIN.top + INNER_H - (tick / chartMax) * INNER_H
  }

  function cloudColor(rank: number): string {
    if (rank < 10) return 'var(--color-text-primary)'
    if (rank < 25) return 'var(--color-accent)'
    return 'var(--color-text-secondary)'
  }

  // Defensive dedupe: upsert guarantees one row per asset, but if older rows
  // survive a migration we keep only the newest text per asset.
  function dedupeByAsset(
    rows: Array<{ assetId: string; textContent: string; createdAt: number }>
  ): string[] {
    const latest = new Map<string, { textContent: string; createdAt: number }>()
    for (const row of rows) {
      const existing = latest.get(row.assetId)
      if (!existing || row.createdAt > existing.createdAt) {
        latest.set(row.assetId, row)
      }
    }
    return [...latest.values()].map((row) => row.textContent)
  }

  async function loadCorpus() {
    const requestToken = fetchGuard.next()
    status = 'loading'
    try {
      const store = getStore()
      const [extractionRows, transcriptionRows] = await Promise.all([
        store.extractions.findTextByCollection(collectionId),
        store.transcriptions.findTextByCollection(collectionId),
      ])
      if (!fetchGuard.isCurrent(requestToken)) return
      corpus = [
        ...dedupeByAsset(extractionRows).map((text) => ({
          text,
          kind: 'extraction' as const,
        })),
        ...dedupeByAsset(transcriptionRows).map((text) => ({
          text,
          kind: 'transcription' as const,
        })),
      ]
    } catch (e) {
      if (!fetchGuard.isCurrent(requestToken)) return
      console.error('[CollectionAnalysisPanel] Failed to load corpus:', e)
      corpus = null
      frequencies = []
      status = 'error'
    }
  }

  async function recompute(texts: CorpusText[], customStopwords: string[]) {
    const requestToken = computeGuard.next()
    const stopwords =
      customStopwords.length > 0 ? new Set([...DEFAULT_STOPWORDS, ...customStopwords]) : undefined
    const result = await buildFrequenciesAsync(texts, stopwords ? { stopwords } : undefined)
    if (!computeGuard.isCurrent(requestToken)) return
    frequencies = result
    status = 'ready'
  }

  $effect(() => {
    void refreshToken
    if (collectionId !== activeSettingsId) {
      activeSettingsId = collectionId
      const loaded = loadAnalysisSettings(collectionId)
      settings = loaded
      stopwordsInput = loaded.customStopwords.join(', ')
    }
    void loadCorpus()
  })

  $effect(() => {
    const texts = corpus
    const customStopwords = settings.customStopwords
    if (texts === null) return
    void recompute(texts, customStopwords)
  })

  function onTermCountChange(event: Event) {
    const input = event.currentTarget as HTMLInputElement
    const next = { ...settings, cloudTermCount: clampCloudTermCount(Number(input.value)) }
    input.value = String(next.cloudTermCount)
    settings = next
    saveAnalysisSettings(collectionId, next)
  }

  function applyStopwords(value: string) {
    const next = { ...settings, customStopwords: parseStopwordsInput(value) }
    settings = next
    saveAnalysisSettings(collectionId, next)
  }

  function onStopwordsInput(event: Event) {
    const value = (event.currentTarget as HTMLTextAreaElement).value
    stopwordsInput = value
    clearTimeout(stopwordsTimer)
    stopwordsTimer = setTimeout(() => applyStopwords(value), 300)
  }

  function inlineSvgStyles(sourceSvg: SVGSVGElement, clonedSvg: SVGSVGElement) {
    const sourceElements = [sourceSvg, ...sourceSvg.querySelectorAll('*')]
    const clonedElements = [clonedSvg, ...clonedSvg.querySelectorAll('*')]
    const styleProperties = [
      'fill',
      'stroke',
      'stroke-width',
      'font-family',
      'font-size',
      'font-weight',
      'text-anchor',
      'dominant-baseline',
    ]

    sourceElements.forEach((sourceElement, index) => {
      const clonedElement = clonedElements[index]
      if (!(clonedElement instanceof SVGElement)) return
      const computed = getComputedStyle(sourceElement)
      for (const property of styleProperties) {
        const value = computed.getPropertyValue(property)
        if (value) clonedElement.style.setProperty(property, value)
      }
    })
  }

  async function svgToPngBytes(sourceSvg: SVGSVGElement): Promise<Uint8Array> {
    const clonedSvg = sourceSvg.cloneNode(true) as SVGSVGElement
    clonedSvg.setAttribute('xmlns', 'http://www.w3.org/2000/svg')
    clonedSvg.setAttribute('width', sourceSvg.viewBox.baseVal.width.toString())
    clonedSvg.setAttribute('height', sourceSvg.viewBox.baseVal.height.toString())
    inlineSvgStyles(sourceSvg, clonedSvg)

    const svgText = new XMLSerializer().serializeToString(clonedSvg)
    const svgBlob = new Blob([svgText], { type: 'image/svg+xml;charset=utf-8' })
    const url = URL.createObjectURL(svgBlob)

    try {
      const image = new Image()
      const imageLoaded = new Promise<void>((resolve, reject) => {
        image.onload = () => resolve()
        image.onerror = () => reject(new Error('Could not render analysis chart'))
      })
      image.src = url
      await imageLoaded

      const canvas = document.createElement('canvas')
      const outputWidth = sourceSvg.viewBox.baseVal.width * PNG_EXPORT_SCALE
      const outputHeight = sourceSvg.viewBox.baseVal.height * PNG_EXPORT_SCALE
      canvas.width = outputWidth
      canvas.height = outputHeight
      const context = canvas.getContext('2d')
      if (!context) throw new Error('Canvas is not available')
      context.imageSmoothingEnabled = true
      context.imageSmoothingQuality = 'high'
      context.drawImage(image, 0, 0, outputWidth, outputHeight)

      const pngBlob = await new Promise<Blob>((resolve, reject) => {
        canvas.toBlob((blob) => {
          if (blob) resolve(blob)
          else reject(new Error('Could not encode analysis chart'))
        }, 'image/png')
      })

      return new Uint8Array(await pngBlob.arrayBuffer())
    } finally {
      URL.revokeObjectURL(url)
    }
  }

  async function downloadChartPng(sourceSvg: SVGSVGElement | undefined, filename: string) {
    if (!sourceSvg) return
    const filePath = await save({
      defaultPath: filename,
      filters: [{ name: 'PNG', extensions: ['png'] }],
    })
    if (!filePath) return
    await writeFile(filePath, await svgToPngBytes(sourceSvg))
  }

  onDestroy(() => {
    clearTimeout(stopwordsTimer)
  })
</script>

<Panel variant="default" padding="none" class="analysis-panel">
  <header class="analysis-header">
    <span class="analysis-header__eyebrow">{translate('collectionAnalysis.eyebrow')}</span>
    <h2 class="analysis-header__title">{translate('collectionAnalysis.title')}</h2>
    {#if status === 'ready' && frequencies.length > 0}
      <p class="analysis-header__meta">
        {translate('collectionAnalysis.meta', {
          words: frequencies.length,
          tokens: totalTokens,
        })}
      </p>
    {/if}
  </header>

  <TabList class="analysis-panel-tabs" aria-label={translate('collectionAnalysis.tabsAria')}>
    <TabButton
      active={activeTab === 'viz'}
      class="analysis-panel-tab"
      onclick={() => {
        activeTab = 'viz'
      }}
    >
      {translate('collectionAnalysis.vizTab')}
    </TabButton>
    <TabButton
      active={activeTab === 'settings'}
      class="analysis-panel-tab"
      onclick={() => {
        activeTab = 'settings'
      }}
    >
      {translate('collectionAnalysis.settingsTab')}
    </TabButton>
  </TabList>

  <div class="analysis-content">
    <div class="analysis-pane" class:is-hidden={activeTab !== 'viz'}>
      {#if status === 'loading'}
        <p class="surface-message surface-message--center">
          {translate('collectionAnalysis.loading')}
        </p>
      {:else if status === 'error'}
        <div class="analysis-error">
          <p class="surface-message surface-message--error">
            {translate('collectionAnalysis.error')}
          </p>
          <Button variant="secondary" size="sm" onclick={() => void loadCorpus()}>
            {translate('collectionAnalysis.retry')}
          </Button>
        </div>
      {:else if frequencies.length === 0}
        <div class="surface-message surface-message--center">
          <p>
            {settings.customStopwords.length > 0 && (corpus?.length ?? 0) > 0
              ? translate('collectionAnalysis.emptyFiltered')
              : translate('collectionAnalysis.empty')}
          </p>
        </div>
      {:else}
        <section class="analysis-section">
          <div class="analysis-section__header">
            <h3 class="analysis-section__title">
              {translate('collectionAnalysis.cloudTitle', { count: settings.cloudTermCount })}
            </h3>
            <IconButton
              class="analysis-section__download"
              size="sm"
              variant="secondary"
              label={translate('collectionAnalysis.downloadCloud')}
              title={translate('collectionAnalysis.downloadCloud')}
              onclick={() =>
                void downloadChartPng(wordCloudSvgEl, `entropia-word-cloud-${collectionId}.png`)}
            >
              <ActionIcon name="download" size={14} />
            </IconButton>
          </div>
          <svg
            bind:this={wordCloudSvgEl}
            viewBox="0 0 {CLOUD_W} {CLOUD_H}"
            preserveAspectRatio="xMidYMid meet"
            class="word-cloud"
            role="img"
            aria-label={translate('collectionAnalysis.cloudTitle', {
              count: settings.cloudTermCount,
            })}
          >
            {#each placedWords as placedWord, rank (placedWord.word)}
              <g>
                <title>{placedWord.word}: {placedWord.count}</title>
                <text
                  class="word-cloud__word"
                  x={placedWord.x}
                  y={placedWord.y}
                  font-size={placedWord.fontSize}
                  font-family={CLOUD_FONT_STACK}
                  font-weight={CLOUD_FONT_WEIGHT}
                  fill={cloudColor(rank)}
                  text-anchor="middle"
                  dominant-baseline="middle"
                  transform={placedWord.rotated
                    ? `rotate(-90 ${placedWord.x} ${placedWord.y})`
                    : undefined}>{placedWord.word}</text
                >
              </g>
            {/each}
          </svg>
        </section>

        <section class="analysis-section">
          <div class="analysis-section__header">
            <h3 class="analysis-section__title">{translate('collectionAnalysis.barsTitle')}</h3>
            <IconButton
              class="analysis-section__download"
              size="sm"
              variant="secondary"
              label={translate('collectionAnalysis.downloadBars')}
              title={translate('collectionAnalysis.downloadBars')}
              onclick={() =>
                void downloadChartPng(barChartSvgEl, `entropia-word-bars-${collectionId}.png`)}
            >
              <ActionIcon name="download" size={14} />
            </IconButton>
          </div>
          <svg
            bind:this={barChartSvgEl}
            viewBox="0 0 {CHART_W} {CHART_H}"
            preserveAspectRatio="xMidYMid meet"
            class="bar-chart"
            role="img"
            aria-label={translate('collectionAnalysis.barsTitle')}
          >
            {#each ticks as tick (tick)}
              <line
                class="bar-chart__gridline"
                x1={MARGIN.left}
                x2={CHART_W - MARGIN.right}
                y1={tickY(tick)}
                y2={tickY(tick)}
              />
              <text class="bar-chart__tick-label" x={MARGIN.left - 6} y={tickY(tick) + 3}>
                {tick}
              </text>
            {/each}
            {#each top20 as { word, count }, i (word)}
              {@const slotX = MARGIN.left + i * SLOT_W}
              {@const labelX = slotX + SLOT_W / 2}
              {@const labelY = MARGIN.top + INNER_H + 12}
              <g>
                <title>{word}: {count}</title>
                <rect
                  class="bar-chart__bar"
                  x={slotX + (SLOT_W - BAR_W) / 2}
                  y={MARGIN.top + INNER_H - barHeight(count)}
                  width={BAR_W}
                  height={barHeight(count)}
                  rx="2"
                />
              </g>
              <text
                class="bar-chart__x-label"
                x={labelX}
                y={labelY}
                transform="rotate(-45 {labelX} {labelY})"
              >
                {truncateLabel(word, 12)}
              </text>
            {/each}
          </svg>
        </section>
      {/if}
    </div>

    <div class="analysis-pane settings-pane" class:is-hidden={activeTab !== 'settings'}>
      <div class="settings-field">
        <label class="settings-label" for="analysis-term-count">
          {translate('collectionAnalysis.termCountLabel')}
        </label>
        <input
          id="analysis-term-count"
          class="settings-input"
          type="number"
          min={MIN_CLOUD_TERMS}
          max={MAX_CLOUD_TERMS}
          value={settings.cloudTermCount}
          onchange={onTermCountChange}
        />
        <p class="settings-hint">
          {translate('collectionAnalysis.termCountHint', {
            min: MIN_CLOUD_TERMS,
            max: MAX_CLOUD_TERMS,
          })}
        </p>
      </div>

      <div class="settings-field">
        <label class="settings-label" for="analysis-stopwords">
          {translate('collectionAnalysis.stopwordsLabel')}
        </label>
        <textarea
          id="analysis-stopwords"
          class="settings-textarea"
          rows="6"
          placeholder={translate('collectionAnalysis.stopwordsPlaceholder')}
          value={stopwordsInput}
          oninput={onStopwordsInput}
        ></textarea>
        <p class="settings-hint">{translate('collectionAnalysis.stopwordsHelp')}</p>
      </div>
    </div>
  </div>
</Panel>

<style>
  :global(.panel.analysis-panel) {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    overflow: hidden;
    padding: 0;
    min-height: 0;
  }

  .analysis-header {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--color-border-subtle);
  }

  .analysis-header__eyebrow {
    font-family: var(--font-mono);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-normal);
    letter-spacing: 0.15em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  .analysis-header__title {
    font-family: var(--font-display);
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-bold);
    color: var(--color-text-primary);
  }

  .analysis-header__meta {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  :global(.analysis-panel-tabs) {
    display: flex;
    flex-wrap: wrap;
    align-self: stretch;
    margin: 0 var(--space-3);
    background: var(--surface-input);
    border-color: var(--border-subtle);
  }

  :global(.analysis-panel-tab) {
    flex: 1 1 auto;
    min-width: fit-content;
  }

  .analysis-content {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 0 var(--space-3) var(--space-3);
    padding: var(--space-2);
  }

  .analysis-pane {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .analysis-pane.is-hidden {
    display: none;
  }

  .analysis-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .analysis-section__title {
    margin: 0;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-secondary);
  }

  .analysis-section__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  :global(.analysis-section__download) {
    width: 28px;
    height: 28px;
  }

  .analysis-error {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
  }

  .word-cloud {
    width: 100%;
    height: auto;
  }

  .word-cloud__word {
    cursor: default;
  }

  .bar-chart {
    width: 100%;
    height: auto;
  }

  .bar-chart__gridline {
    stroke: var(--color-hairline);
    stroke-width: 1;
  }

  .bar-chart__tick-label {
    fill: var(--color-text-muted);
    font-size: 9px;
    text-anchor: end;
    font-variant-numeric: tabular-nums;
  }

  .bar-chart__bar {
    fill: var(--color-accent);
  }

  .bar-chart__x-label {
    fill: var(--color-text-secondary);
    font-size: 9px;
    text-anchor: end;
  }

  .settings-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .settings-label {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-bold);
    color: var(--color-text-primary);
  }

  .settings-input,
  .settings-textarea {
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    font-family: inherit;
  }

  .settings-input {
    max-width: 120px;
    font-variant-numeric: tabular-nums;
  }

  .settings-textarea {
    resize: vertical;
    min-height: 96px;
  }

  .settings-input:focus-visible,
  .settings-textarea:focus-visible {
    outline: none;
    border-color: var(--border-focus);
    box-shadow: var(--focus-ring);
  }

  .settings-hint {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }
</style>
