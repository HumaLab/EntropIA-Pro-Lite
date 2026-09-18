<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import {
    ActionIcon,
    Button,
    ConfirmDialog,
    IconButton,
    Panel,
    StatusBadge,
    WritingEditor,
    outlineDepth,
    outlineFromDocument,
  } from '@entropia/ui'
  import type { CanonicalDocument, StatusBadgeVariant } from '@entropia/ui'
  import type { SuggestionRow } from '$lib/writing-agent'
  import { settingsGet, settingsSet, SETTINGS_KEYS } from '$lib/settings'
  // Shared with the exporter on purpose: two copies of this mapping would let
  // the manuscript on screen and the exported file cite the same works
  // differently, with nothing reporting it.
  import { clusterOf } from '$lib/citation-clusters'
  import { plainTextOf } from '$lib/note-text'
  import WritingExportDialog from './WritingExportDialog.svelte'
  import {
    ResizeHandle,
    EDITOR_MIN_WIDTH,
    OUTLINE_BOUNDS,
    RESEARCH_BOUNDS,
    readPanelWidth,
  } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { navigation, type View } from '$lib/navigation'
  import { writing, type SaveStatus, type WritingDocumentRow } from '$lib/writing'
  import { getStore } from '$lib/db'
  import { resolveCitationTarget, type CitationTarget } from '$lib/citation-target'
  import { writingNotes } from '$lib/writing-notes'
  import { resolveNoteLink, type NoteLinkState } from '$lib/note-link'
  import WritingCitationDialog from './WritingCitationDialog.svelte'
  import { DEFAULT_STYLE, isCslError, renderDocument } from '$lib/writing-csl'
  import { tooltip, worksOf } from '@entropia/ui'
  import WritingResearchPanel, { type ResearchTab } from './WritingResearchPanel.svelte'

  const store = writing
  let snapshot = $state(store.snapshot)
  const unsubscribe = store.subscribe((value) => {
    snapshot = value
  })

  // `subscribe` fires synchronously, so this is populated before first use.
  let navSnapshot = $state<{ current: View; canGoBack: boolean } | null>(null)
  const unsubscribeNav = navigation.subscribe((value) => {
    navSnapshot = value
  })

  /**
   * The one place the store's open document is decided.
   *
   * Every route into this section — a card, the Back button, the section
   * crumb, the top-bar icon — changes navigation and nothing else. This
   * reconciles the store to it. Anything that also mutated the store directly
   * would race against this and lose.
   */
  let reconciling = false
  $effect(() => {
    const view = navSnapshot?.current
    const requested = view?.name === 'writing' ? (view.documentId ?? null) : null
    const openId = snapshot.open?.id ?? null
    if (!snapshot.ready || reconciling) return
    if (requested === openId) return

    reconciling = true
    void (async () => {
      try {
        if (requested) {
          await store.openDocument(requested)
        } else {
          store.closeDocument()
          await store.listDocuments()
        }
      } finally {
        reconciling = false
      }
    })()
  })

  /**
   * Whether the agent can be asked anything at all (§14.1).
   *
   * One question, answered from the settings: is there a credential. Anything
   * more — whether the model is reachable, whether it has quota — cannot be
   * known without spending a request, and guessing would mean offering actions
   * that fail once the writer has already chosen a passage.
   */
  let hasChatModel = $state(false)

  /**
   * Whether corpus retrieval can run, for §14.1's four evidence actions.
   *
   * The same credential, and that is the whole of it: G9's limit is that
   * `ClienteEmbeddings` and `ClienteRerank` are concrete types bound to
   * OpenRouter, so the *provider* cannot be swapped the way the chat model's
   * can — not that retrieval is unavailable. With the credential present it
   * runs, which is the same path Chat/RAG already takes.
   *
   * It was left hardcoded false when the panel was first wired, so the four
   * actions sat greyed out on machines where retrieval worked.
   */
  const hasRetrieval = $derived(hasChatModel)

  onMount(async () => {
    // Only the gate and the list. Which document is open is the effect's
    // business, including on a remount that arrives with one still held: the
    // store is a module singleton and outlives this view.
    if (await store.init()) await store.listDocuments()
    await loadPanelWidths()
    try {
      hasChatModel = Boolean((await settingsGet(SETTINGS_KEYS.OPENROUTER_API_KEY))?.trim())
    } catch {
      // A settings read that failed is not a credential. The panel says the
      // agent is unavailable, and writing by hand carries on untouched.
      hasChatModel = false
    }
  })

  onDestroy(() => {
    unsubscribe()
    unsubscribeNav()
    // Persist whatever is pending, then release the timer. The document stays
    // open in the store on purpose: navigating away and back should return to
    // it, and onMount reconciles against navigation.
    void store.flush().finally(() => store.dispose())
  })

  const STATUS_LABEL: Record<SaveStatus, string> = {
    saved: 'writing.status.saved',
    saving: 'writing.status.saving',
    pending: 'writing.status.pending',
    error: 'writing.status.error',
    'recovery-available': 'writing.status.recovery',
  }

  function statusVariant(status: SaveStatus): StatusBadgeVariant {
    if (status === 'saved') return 'success'
    if (status === 'error') return 'danger'
    if (status === 'pending' || status === 'recovery-available') return 'warning'
    return 'neutral'
  }

  async function createDocument() {
    const id = await store.createDocument(t('writing.newDocumentTitle'))
    if (id) open(id)
  }

  /**
   * Opening is a navigation, nothing else.
   *
   * The reconciling effect below owns the store: if this also called
   * `openDocument` the two would race — the store would change first, the
   * effect would see a route with no document yet and close it again.
   *
   * Pushed, not replaced: the list has to stay in history so the shell's Back
   * button returns to it instead of leaving the section entirely.
   */
  function open(id: string) {
    navigation.navigate({
      name: 'writing',
      documentId: id,
      documentTitle: documents.find((d) => d.id === id)?.title ?? null,
    })
  }

  async function backToList() {
    await store.flush()
    if (navigationCanGoBack()) {
      navigation.back()
    } else {
      navigation.replace({ name: 'writing', documentId: null, documentTitle: null })
    }
  }

  function navigationCanGoBack(): boolean {
    return navSnapshot?.canGoBack ?? false
  }

  async function commitTitle(value: string) {
    const current = snapshot.open
    if (!current || value.trim() === current.title) return
    await store.renameDocument(current.id, value)
    navigation.replace({
      name: 'writing',
      documentId: current.id,
      documentTitle: snapshot.open?.title ?? null,
    })
  }

  function onTitleKeydown(event: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    if (event.key === 'Enter') {
      event.preventDefault()
      event.currentTarget.blur()
    }
    if (event.key === 'Escape') {
      event.currentTarget.value = snapshot.open?.title ?? ''
      event.currentTarget.blur()
    }
  }

  function onEditorChange(next: CanonicalDocument) {
    store.applyEdit(next)
  }

  function formatDate(ms: number): string {
    return new Date(ms).toLocaleString()
  }

  let editorRef = $state<
    | {
        goToPosition: (position: number) => void
        renameOutlineSection: (childIndex: number, title: string) => boolean
        deleteOutlineSection: (childIndex: number) => boolean
        moveOutlineSection: (childIndex: number, direction: 1 | -1) => boolean
        addSectionAfter: (childIndex: number, title?: string) => boolean
        weighSection: (childIndex: number) => { words: number; headings: number }
        insertCitation: (attrs: Record<string, unknown>) => string | null
        selectedText: () => string
        insertNoteText: (text: string) => boolean
        insertNoteLink: (attrs: Record<string, unknown>) => string | null
        insertZoteroCitation: (attrs: Record<string, unknown>) => string | null
        zoteroCitations: () => { id: string; attrs: Record<string, unknown> }[]
        updateZoteroCitation: (id: string, attrs: Record<string, unknown>) => boolean
        passageStillThere: (passage: string) => boolean
        replaceWithSuggestion: (passage: string, proposal: string) => boolean
        insertSuggestionBelow: (passage: string, proposal: string) => boolean
      }
    | undefined
  >(undefined)

  /**
   * Following a note link back to its note (§13).
   *
   * Two things happen, in this order. First the link is checked against the
   * note as it stands now, because a writer about to read a note should be
   * told if it no longer says what their manuscript quotes. Then the item is
   * opened with that note showing.
   *
   * The manuscript is never touched. §13 forbids a diverged note overwriting
   * the article, and §13.1 forbids a deleted one removing the text or the
   * snapshot — so the new text is *shown*, never applied.
   */
  let noteNotice = $state<{ state: NoteLinkState; attrs: Record<string, unknown> } | null>(null)

  /**
   * Whether the export panel is open (§17). Closed by default: exporting is a
   * deliberate act, not something to trip over while writing.
   */
  let exporting = $state(false)

  /**
   * How wide each side panel is (§18: "paneles redimensionables y plegables").
   *
   * Folding was already a button; this is the other half. It matters most at the
   * sizes the window actually reaches — 900px is its floor and the zoom ceiling
   * is 125%, which leaves about 720 CSS pixels for three columns — because that
   * is where the manuscript ends up narrower than the panels beside it. Fixed
   * widths made that the layout's decision; now it is the writer's.
   *
   * Persisted, because a width someone chose and has to choose again every
   * session is not a setting, it is a chore.
   */
  let outlineWidth = $state(OUTLINE_BOUNDS.initial)
  let researchWidth = $state(RESEARCH_BOUNDS.initial)

  async function loadPanelWidths() {
    try {
      const [outline, research] = await Promise.all([
        settingsGet(SETTINGS_KEYS.WRITING_OUTLINE_WIDTH),
        settingsGet(SETTINGS_KEYS.WRITING_RESEARCH_WIDTH),
      ])
      outlineWidth = readPanelWidth(outline, OUTLINE_BOUNDS)
      researchWidth = readPanelWidth(research, RESEARCH_BOUNDS)
    } catch {
      // A settings read that failed is not a reason to show no panels. They
      // open at their usual width and the session works as it always did.
    }
  }

  function persistWidth(key: string, width: number) {
    // Not awaited: the panel has already moved, and a write that loses a race
    // with the next drag costs a remembered width, not the width on screen.
    void settingsSet(key, String(width)).catch(() => {})
  }

  async function followNoteLink(attrs: Record<string, unknown>) {
    const noteId = readString(attrs.noteId)
    const today = noteId ? await writingNotes.readNote(noteId) : { exists: false, content: null }

    const state = await resolveNoteLink(
      {
        noteId,
        contentSnapshot: readString(attrs.contentSnapshot),
        contentHash: readString(attrs.contentHash),
      },
      today
    )

    if (state.integrity !== 'valid') noteNotice = { state, attrs }
    // A note that is gone has nowhere to take the writer, so the notice is all
    // there is. Every other outcome still opens it.
    if (state.integrity === 'source_missing') return

    const itemId = readString(attrs.itemId)
    if (!itemId || !noteId) return
    const item = await getStore().items.findById(itemId)
    if (!item) return

    await store.flush()
    navigation.navigate({
      name: 'item',
      collectionId: item.collectionId,
      collectionName: '',
      itemId,
      itemTitle: item.title,
      noteId,
    })
  }

  /**
   * The citation being adjusted, held by identity rather than by position.
   *
   * A position would be stale the moment anything above it changed, and the
   * panel stays open while the writer keeps typing.
   */
  let editingCitation = $state<{ id: string; attrs: Record<string, unknown> } | null>(null)

  /** The cluster's works, in a shape the dialog can edit one by one. */
  function citationItems(attrs: Record<string, unknown>) {
    return worksOf({ attrs }).map((raw) => {
      const item = (raw ?? {}) as Record<string, unknown>
      return {
        itemKey: readString(item.itemKey) ?? '',
        title: readString((item.metadataTitle ?? '') as string) ?? '',
        snapshot:
          typeof item.metadataSnapshot === 'string'
            ? item.metadataSnapshot
            : JSON.stringify(item.metadataSnapshot ?? {}),
        locator: readString(item.locator) ?? '',
        locatorType: readString(item.locatorType) ?? 'page',
        suppressAuthor: item.suppressAuthor === true,
      }
    })
  }

  function citationAffixes(attrs: Record<string, unknown>) {
    return {
      prefix: readString(attrs.prefix) ?? '',
      suffix: readString(attrs.suffix) ?? '',
    }
  }

  /**
   * Renders every citation in the manuscript, together (§11.5, §11.6).
   *
   * Together is not an optimisation, it is the requirement. Disambiguation is a
   * property of the document — which of two works by one author in one year
   * reads `2015a` depends on all the others — so a citation rendered on its own
   * can only ever say `2015`, twice.
   *
   * It is also what a change of style means: each citation's text is derived
   * from its stored CSL data, so re-deriving is the whole operation. Had the
   * rendering been the stored truth, the old strings would simply stay.
   */
  let renderingCitations = false
  async function renderAllCitations() {
    // One pass at a time. Each pass writes onto the nodes it just read, and a
    // second pass reading them mid-flight would render from half-updated ones.
    if (renderingCitations) return
    const citations = editorRef?.zoteroCitations() ?? []
    if (citations.length === 0) return

    renderingCitations = true
    try {
      const rendered = await renderDocument(
        citations.map((citation) => clusterOf(citation.attrs)),
        DEFAULT_STYLE
      )
      // A manuscript that will not render keeps whatever it was showing.
      // Blanking it would turn a style problem into a document that looks
      // damaged.
      if (isCslError(rendered)) return
      citations.forEach((citation, index) => {
        const text = rendered[index]?.text
        if (text) editorRef?.updateZoteroCitation(citation.id, { renderedText: text })
      })
    } finally {
      renderingCitations = false
    }
  }

  /**
   * Puts a bibliographic citation in and records where it came from (§11.5).
   *
   * The snapshot travels on the node, which is what lets §11.3 render the
   * citation with Zotero closed and what survives the work being deleted from
   * the library.
   */
  function citeZotero(attrs: Record<string, unknown>): string | null {
    const citationNodeId = editorRef?.insertZoteroCitation(attrs) ?? null
    if (!citationNodeId) return null
    store.queueProvenance({
      id: crypto.randomUUID(),
      origin_type: 'zotero',
      operation_type: 'insert',
      range_anchor_json: JSON.stringify({ citationNodeId }),
      source_reference_json: JSON.stringify({ itemKey: attrs.itemKey ?? null }),
      model_provider: null,
      model_name: null,
    })
    // The whole manuscript, not just this citation: adding a work can change
    // how another one reads, because two works by one author in one year are
    // told apart by letters that depend on all of them.
    void renderAllCitations()
    return citationNodeId
  }

  function editCitation(attrs: Record<string, unknown>) {
    const id = typeof attrs.citationNodeId === 'string' ? attrs.citationNodeId : null
    if (id) editingCitation = { id, attrs }
  }

  /**
   * The two ways a note can enter the manuscript (§13).
   *
   * Copying records no provenance event, and that is not an oversight: the
   * text becomes the writer's own and has no relationship to the note. Linking
   * records one, because a link is a live relationship that can later diverge.
   */
  function copyNoteText(text: string): boolean {
    return editorRef?.insertNoteText(text) ?? false
  }

  /**
   * Putting an accepted agent proposal into the manuscript (§14.2, §14.5).
   *
   * The order matters and is not interchangeable. The backend has already
   * decided that this acceptance is the one that applies — a second click
   * arrives here with nothing to apply — so by the time this runs the record
   * says the suggestion is resolved. What is left is to write the text and to
   * record where it came from.
   *
   * The provenance event is what §14.5 asks for: a reader of the manuscript
   * should be able to ask which passages were written with the agent, with
   * which model, and on what evidence. So the event carries the provider, the
   * model and the suggestion's identity, and `source_reference_json` points at
   * the suggestion row rather than copying its evidence — the row is the
   * record, and two copies of it would eventually disagree.
   *
   * If the text could not be written — the target moved between the check and
   * the write — no event is recorded. A provenance log that claims an edit that
   * never happened is worse than none.
   */
  function applySuggestion(suggestion: SuggestionRow, text: string, below: boolean) {
    const passage = suggestion.original_text ?? ''
    const written = below
      ? (editorRef?.insertSuggestionBelow(passage, text) ?? false)
      : (editorRef?.replaceWithSuggestion(passage, text) ?? false)
    if (!written) return

    store.queueProvenance({
      id: crypto.randomUUID(),
      origin_type: 'agent',
      operation_type: below ? 'insert' : 'replace',
      range_anchor_json: suggestion.selection_anchor_json,
      source_reference_json: JSON.stringify({
        suggestionId: suggestion.id,
        actionType: suggestion.action_type,
        sourceRevision: suggestion.source_revision,
      }),
      model_provider: suggestion.provider,
      model_name: suggestion.model,
    })
  }

  function linkNote(attrs: Record<string, unknown>): string | null {
    const noteLinkNodeId = editorRef?.insertNoteLink(attrs) ?? null
    if (!noteLinkNodeId) return null
    store.queueProvenance({
      id: crypto.randomUUID(),
      origin_type: 'note',
      operation_type: 'insert',
      range_anchor_json: JSON.stringify({ noteLinkNodeId }),
      source_reference_json: JSON.stringify({
        noteId: attrs.noteId ?? null,
        itemId: attrs.itemId ?? null,
        contentHash: attrs.contentHash ?? null,
      }),
      model_provider: null,
      model_name: null,
    })
    return noteLinkNodeId
  }

  /**
   * Following a citation back to its source (§10.2).
   *
   * The five steps in order: resolve the asset, open the viewer, go to the
   * page, highlight the range, and — when the anchor no longer resolves — show
   * the fragment and the metadata the citation recorded instead.
   *
   * Nothing here ever removes or rewrites the citation. §10.3 is explicit that
   * a citation outlives its source, which is why its corpus ids are snapshots
   * with no foreign key behind them.
   */
  let sourceNotice = $state<{ target: CitationTarget; attrs: Record<string, unknown> } | null>(null)

  function readString(value: unknown): string | null {
    return typeof value === 'string' && value.length > 0 ? value : null
  }

  function readNumber(value: unknown): number | null {
    return typeof value === 'number' && Number.isFinite(value) ? value : null
  }

  async function followCitation(attrs: Record<string, unknown>) {
    const assetId = readString(attrs.assetId)
    const store$ = getStore()

    let assetExists = false
    let extractedText: string | null = null
    if (assetId) {
      const asset = await store$.assets.findById(assetId)
      assetExists = asset !== null
      if (assetExists) {
        extractedText = (await store$.extractions.findByAsset(assetId))?.textContent ?? null
      }
    }

    const target = await resolveCitationTarget(
      {
        assetId,
        pageNumber: readNumber(attrs.pageNumber),
        startChar: readNumber(attrs.startChar),
        endChar: readNumber(attrs.endChar),
        quotedText: readString(attrs.quotedText),
        sourceTextHash: readString(attrs.sourceTextHash),
      },
      { assetExists, extractedText }
    )

    // A source that is gone, or one that no longer says what was cited, is
    // reported rather than navigated to: opening a page to show the wrong
    // sentence is worse than saying so.
    if (!target.canOpen || target.integrity !== 'valid') {
      sourceNotice = { target, attrs }
      if (!target.canOpen) return
    }

    const itemId = readString(attrs.itemId)
    const collectionId = readString(attrs.collectionId)
    if (!itemId || !collectionId || !target.assetId) return
    const item = await store$.items.findById(itemId)
    if (!item) return

    await store.flush()
    navigation.navigate({
      name: 'item',
      collectionId,
      collectionName: '',
      itemId,
      itemTitle: item.title,
      assetId: target.assetId,
      citationRange:
        target.canHighlight && target.start !== null && target.end !== null
          ? {
              start: target.start,
              end: target.end,
              text: readString(attrs.quotedText) ?? '',
            }
          : null,
    })
  }

  /**
   * Puts a corpus citation in the manuscript and records where it came from.
   *
   * The node and the provenance event are raised together but committed
   * together too: the event only waits in the store until the save that carries
   * the edit commits both in one transaction (§8.4, §10.1). Nothing is written
   * here, so a failure leaves the draft and the error rather than a half-
   * confirmed citation.
   *
   * The range anchor stores the citation's identity, not a document position.
   * Spike S2 measured that a persisted `{from, to}` points at a different
   * paragraph after a reload, while identity resolves correctly.
   */
  function insertCorpusCitation(attrs: Record<string, unknown>): string | null {
    const citationNodeId = editorRef?.insertCitation(attrs) ?? null
    if (!citationNodeId) return null
    store.queueProvenance({
      id: crypto.randomUUID(),
      origin_type: 'corpus',
      // The operation is an insertion; that it came from the corpus is what
      // `origin_type` says. Both are constrained by a CHECK in the migration.
      operation_type: 'insert',
      range_anchor_json: JSON.stringify({ citationNodeId }),
      source_reference_json: JSON.stringify({
        collectionId: attrs.collectionId ?? null,
        itemId: attrs.itemId ?? null,
        assetId: attrs.assetId ?? null,
        pageNumber: attrs.pageNumber ?? null,
        startChar: attrs.startChar ?? null,
        endChar: attrs.endChar ?? null,
        sourceTextHash: attrs.sourceTextHash ?? null,
      }),
      model_provider: null,
      model_name: null,
    })
    return citationNodeId
  }

  /**
   * The section a delete was asked for, held until it is confirmed.
   *
   * Moves and renames are not confirmed: they are visible the instant they
   * happen, so a mistake announces itself. A delete does the opposite — the
   * outline closes over the gap and the writing continues, and by the time the
   * loss is noticed the undo history has moved on and autosave has persisted
   * it. This is the one that needs asking.
   */
  let pendingSectionDelete = $state<{ title: string; childIndex: number } | null>(null)
  let pendingSectionWeight = $state({ words: 0, headings: 0 })

  function askDeleteSection(childIndex: number, title: string) {
    pendingSectionWeight = editorRef?.weighSection(childIndex) ?? { words: 0, headings: 0 }
    pendingSectionDelete = { childIndex, title }
  }

  function confirmDeleteSection() {
    const target = pendingSectionDelete
    pendingSectionDelete = null
    if (target) editorRef?.deleteOutlineSection(target.childIndex)
  }

  /**
   * Which outline entry is being renamed, by its child index.
   *
   * None of the section operations asks for confirmation: each is a single
   * transaction, so Ctrl+Z undoes it whole, and a dialog in front of an
   * undoable action only slows down the person who meant it.
   */
  let renamingSection = $state<number | null>(null)

  function commitSectionName(childIndex: number, title: string) {
    renamingSection = null
    editorRef?.renameOutlineSection(childIndex, title.trim())
  }

  function onSectionNameKeydown(event: KeyboardEvent, childIndex: number) {
    if (event.key === 'Enter') {
      event.preventDefault()
      commitSectionName(
        childIndex,
        event.currentTarget instanceof HTMLInputElement ? event.currentTarget.value : ''
      )
    }
    if (event.key === 'Escape') {
      event.preventDefault()
      renamingSection = null
    }
  }
  /**
   * Which side panels are showing. §6.3 asks for both to be foldable so the
   * editor can take the full width for a concentrated session, so the two are
   * remembered the same way rather than one being a special case.
   */
  const OUTLINE_PREFERENCE = 'entropia-writing-outline'
  const RESEARCH_PREFERENCE = 'entropia-writing-research'

  function readPanelPreference(key: string): boolean {
    try {
      return localStorage.getItem(key) !== 'closed'
    } catch {
      // A blocked storage is not a reason to hide a panel.
      return true
    }
  }

  function writePanelPreference(key: string, open: boolean) {
    try {
      localStorage.setItem(key, open ? 'open' : 'closed')
    } catch {
      // Nor is it a reason to refuse the toggle.
    }
  }

  let outlineOpen = $state(readPanelPreference(OUTLINE_PREFERENCE))
  let researchOpen = $state(readPanelPreference(RESEARCH_PREFERENCE))
  let researchTab = $state<ResearchTab>('corpus')

  /** Derived from the document, never kept as a second copy (§6.1). */
  const outline = $derived(outlineFromDocument(snapshot.content))

  function toggleOutline() {
    outlineOpen = !outlineOpen
    writePanelPreference(OUTLINE_PREFERENCE, outlineOpen)
  }

  function toggleResearch() {
    researchOpen = !researchOpen
    writePanelPreference(RESEARCH_PREFERENCE, researchOpen)
  }

  /** The document a discard was asked for, held until it is confirmed. */
  let pendingDiscard = $state<WritingDocumentRow | null>(null)

  async function confirmDiscard() {
    const target = pendingDiscard
    if (!target) return
    pendingDiscard = null
    await store.trashDocument(target.id)
  }

  /** Recomputed from the snapshot so it tracks every status change. */
  const canRetrySave = $derived(snapshot.status === 'error' && store.canRetrySave)

  const openDocument = $derived(snapshot.open)
  const documents = $derived(snapshot.documents as WritingDocumentRow[])
</script>

<section class="writing">
  {#if !snapshot.ready}
    <Panel padding="lg">
      <p class="writing__notice" role="status">{t('writing.notReady')}</p>
    </Panel>
  {:else if openDocument && (snapshot.content || snapshot.refusal)}
    <header class="writing__bar">
      <Button variant="ghost" size="sm" onclick={backToList}>
        <ActionIcon name="chevron-left" size={14} />
        {t('writing.backToList')}
      </Button>
      <input
        class="writing__doc-title"
        type="text"
        value={openDocument.title}
        aria-label={t('writing.titleLabel')}
        placeholder={t('writing.newDocumentTitle')}
        onblur={(event) => commitTitle(event.currentTarget.value)}
        onkeydown={onTitleKeydown}
      />
      <IconButton
        size="sm"
        variant="ghost"
        label={t('writing.toggleOutline')}
        active={outlineOpen}
        onclick={toggleOutline}
      >
        <ActionIcon name="list" size={14} />
      </IconButton>
      <IconButton
        size="sm"
        variant="ghost"
        label={t('writing.toggleResearch')}
        active={researchOpen}
        onclick={toggleResearch}
      >
        <ActionIcon name="search" size={14} />
      </IconButton>
      <IconButton
        size="sm"
        variant="ghost"
        label={t('writing.exportTitle')}
        active={exporting}
        onclick={() => (exporting = !exporting)}
      >
        <ActionIcon name="download" size={14} />
      </IconButton>
      <div class="writing__bar-end">
        <span class="writing__revision">
          {t('writing.revision', { revision: String(snapshot.revision) })}
        </span>
        <StatusBadge variant={statusVariant(snapshot.status)}>
          {t(STATUS_LABEL[snapshot.status])}
        </StatusBadge>
      </div>
    </header>

    {#if exporting && snapshot.content}
      <!-- Reads `snapshot.content`, which every keystroke already updates, so
           the export is of what is on screen rather than of the last save. -->
      <WritingExportDialog
        doc={snapshot.content.doc}
        title={openDocument.title}
        onclose={() => (exporting = false)}
      />
    {/if}

    {#if editingCitation}
      <!-- Keyed by the citation, so opening a different one starts a fresh copy
           of its settings rather than editing the previous one's. -->
      {#key editingCitation.id}
        <WritingCitationDialog
          items={citationItems(editingCitation.attrs)}
          affixes={citationAffixes(editingCitation.attrs)}
          onapply={(next) => {
            if (!editingCitation) return
            editorRef?.updateZoteroCitation(editingCitation.id, next)
            void renderAllCitations()
          }}
          onclose={() => (editingCitation = null)}
        />
      {/key}
    {/if}

    {#if noteNotice}
      <Panel padding="md">
        <div class="writing__source-notice" role="status">
          <div>
            <p class="writing__source-title">
              {noteNotice.state.integrity === 'source_missing'
                ? t('writing.noteMissingTitle')
                : noteNotice.state.integrity === 'source_modified'
                  ? t('writing.noteChangedTitle')
                  : t('writing.noteUnverifiable')}
            </p>
            {#if noteNotice.state.integrity !== 'unverifiable'}
              <p class="writing__source-body">
                {noteNotice.state.integrity === 'source_missing'
                  ? t('writing.noteMissingBody')
                  : t('writing.noteChangedBody')}
              </p>
            {/if}
            <blockquote class="writing__source-quote">
              <!-- `current` is the note as it is stored, which is markup. The
                   snapshot beside it has already been through the same
                   extraction, so both sides of the comparison read alike. -->
              {plainTextOf(
                noteNotice.state.current ?? String(noteNotice.attrs.contentSnapshot ?? '')
              )}
            </blockquote>
          </div>
          <Button variant="ghost" size="sm" onclick={() => (noteNotice = null)}>
            {t('writing.noteDismiss')}
          </Button>
        </div>
      </Panel>
    {/if}

    {#if sourceNotice}
      <Panel padding="md">
        <div class="writing__source-notice" role="status">
          <div>
            <p class="writing__source-title">
              {sourceNotice.target.integrity === 'source_missing'
                ? t('writing.sourceMissingTitle')
                : sourceNotice.target.integrity === 'source_modified'
                  ? t('writing.sourceModifiedTitle')
                  : t('writing.sourceUnverifiable')}
            </p>
            <p class="writing__source-body">
              {sourceNotice.target.integrity === 'source_missing'
                ? t('writing.sourceMissingBody')
                : t('writing.sourceModifiedBody')}
            </p>
            {#if typeof sourceNotice.attrs.quotedText === 'string'}
              <blockquote class="writing__source-quote">
                {sourceNotice.attrs.quotedText}
              </blockquote>
            {/if}
          </div>
          <Button variant="ghost" size="sm" onclick={() => (sourceNotice = null)}>
            {t('writing.sourceDismiss')}
          </Button>
        </div>
      </Panel>
    {/if}

    {#if snapshot.status === 'error' && snapshot.error}
      <Panel padding="md">
        <div class="writing__save-error" role="alert">
          <p class="writing__error">
            {canRetrySave
              ? t('writing.saveFailed', { message: snapshot.error.message })
              : t('writing.saveConflict')}
          </p>
          {#if canRetrySave}
            <Button variant="secondary" size="sm" onclick={() => void store.retrySave()}>
              <ActionIcon name="refresh" size={14} />
              {t('writing.retrySave')}
            </Button>
          {/if}
        </div>
      </Panel>
    {/if}

    {#if snapshot.repair}
      <Panel padding="md">
        <div class="writing__notice-row">
          <p class="writing__notice" role="status">
            {t('writing.repaired', {
              count: String(snapshot.repair.orphanFootnoteReferences),
            })}
          </p>
          <IconButton
            size="sm"
            variant="ghost"
            label={t('writing.repairedDismiss')}
            title={t('writing.repairedDismiss')}
            onclick={() => writing.dismissRepair()}
          >
            <ActionIcon name="close" size={14} />
          </IconButton>
        </div>
      </Panel>
    {/if}

    <div class="writing__workspace">
      {#if outlineOpen}
        <nav
          class="writing__outline"
          id="writing-outline-panel"
          style:flex-basis="{outlineWidth}px"
          style:min-width="{OUTLINE_BOUNDS.squeeze}px"
          aria-label={t('writing.outline')}
        >
          <p class="writing__outline-title">{t('writing.outline')}</p>
          {#if outline.length === 0}
            <p class="writing__outline-empty">{t('writing.outlineEmpty')}</p>
          {:else}
            <ul class="writing__outline-list">
              {#each outline as entry (entry.index)}
                <li class="writing__outline-row">
                  {#if renamingSection === entry.childIndex}
                    <input
                      class="writing__outline-rename"
                      type="text"
                      value={entry.text}
                      aria-label={t('writing.sectionRename')}
                      onkeydown={(event) => onSectionNameKeydown(event, entry.childIndex)}
                      onblur={(event) =>
                        commitSectionName(entry.childIndex, event.currentTarget.value)}
                      {@attach (node) => node.focus()}
                    />
                  {:else}
                    <!-- The outline narrows to 120px when the window forces
                         it, and a heading that no longer fits is truncated with
                         an ellipsis. A screen reader still reads the whole
                         thing — CSS truncation does not change the text — but
                         someone looking at it has nothing, so the full heading
                         is here to be hovered. -->
                    <button
                      type="button"
                      class="writing__outline-item"
                      use:tooltip={entry.text || t('writing.outlineUntitled')}
                      style:padding-left="calc(var(--space-2) + {outlineDepth(outline, entry)} * var(--space-3))"
                      onclick={() => editorRef?.goToPosition(entry.position)}
                      ondblclick={() => (renamingSection = entry.childIndex)}
                    >
                      {entry.text || t('writing.outlineUntitled')}
                    </button>
                    <span
                      class="writing__outline-actions"
                      role="group"
                      aria-label={t('writing.sectionActions')}
                    >
                      <IconButton
                        size="sm"
                        variant="ghost"
                        label={t('writing.sectionUp')}
                        onclick={() => editorRef?.moveOutlineSection(entry.childIndex, -1)}
                        ><ActionIcon name="chevron-up" size={12} /></IconButton
                      >
                      <IconButton
                        size="sm"
                        variant="ghost"
                        label={t('writing.sectionDown')}
                        onclick={() => editorRef?.moveOutlineSection(entry.childIndex, 1)}
                        ><ActionIcon name="chevron-down" size={12} /></IconButton
                      >
                      <IconButton
                        size="sm"
                        variant="ghost"
                        label={t('writing.sectionRename')}
                        onclick={() => (renamingSection = entry.childIndex)}
                        ><ActionIcon name="edit" size={12} /></IconButton
                      >
                      <IconButton
                        size="sm"
                        variant="ghost"
                        label={t('writing.sectionAdd')}
                        onclick={() => editorRef?.addSectionAfter(entry.childIndex)}
                        ><ActionIcon name="add" size={12} /></IconButton
                      >
                      <IconButton
                        size="sm"
                        variant="ghost"
                        label={t('writing.sectionDelete')}
                        onclick={() =>
                          askDeleteSection(
                            entry.childIndex,
                            entry.text || t('writing.outlineUntitled')
                          )}><ActionIcon name="delete" size={12} /></IconButton
                      >
                    </span>
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
        </nav>
        <ResizeHandle
          width={outlineWidth}
          bounds={OUTLINE_BOUNDS}
          side="start"
          label={t('writing.outlineWidth')}
          controls="writing-outline-panel"
          onresize={(width) => (outlineWidth = width)}
          oncommit={(width) => persistWidth(SETTINGS_KEYS.WRITING_OUTLINE_WIDTH, width)}
        />
      {/if}

      <div class="writing__editor" style:min-width="{EDITOR_MIN_WIDTH}px">
        {#if snapshot.content}
          <WritingEditor
            bind:this={editorRef}
            document={snapshot.content}
            onchange={onEditorChange}
            oncitation={followCitation}
            onnotelink={followNoteLink}
            onzoterocitation={editCitation}
            placeholder={t('writing.placeholder')}
          />
        {:else if snapshot.refusal}
          <WritingEditor document={{ schemaVersion: 1, doc: { type: 'doc' } }} toolbar={false} />
        {/if}
      </div>

      {#if researchOpen}
        <ResizeHandle
          width={researchWidth}
          bounds={RESEARCH_BOUNDS}
          side="end"
          label={t('writing.researchWidth')}
          controls="writing-research-panel"
          onresize={(width) => (researchWidth = width)}
          oncommit={(width) => persistWidth(SETTINGS_KEYS.WRITING_RESEARCH_WIDTH, width)}
        />
        <aside
          class="writing__research"
          id="writing-research-panel"
          style:flex-basis="{researchWidth}px"
          style:min-width="{RESEARCH_BOUNDS.squeeze}px"
        >
          <WritingResearchPanel
            bind:tab={researchTab}
            oninsertcitation={insertCorpusCitation}
            oncopynote={copyNoteText}
            onlinknote={linkNote}
            selection={() => editorRef?.selectedText() ?? ''}
            oncitezotero={citeZotero}
            documentId={openDocument?.id ?? null}
            hasChat={hasChatModel}
            {hasRetrieval}
            sourceRevision={snapshot.revision}
            passagePresent={(passage) => editorRef?.passageStillThere(passage) ?? false}
            onapplysuggestion={applySuggestion}
          />
        </aside>
      {/if}
    </div>
  {:else}
    <header class="writing__header">
      <div>
        <p class="writing__eyebrow">{t('writing.eyebrow')}</p>
        <h1 class="writing__title">{t('writing.title')}</h1>
        <p class="writing__subtitle">{t('writing.subtitle')}</p>
      </div>
      <Button
        variant="primary"
        size="md"
        iconOnly
        aria-label={t('writing.newDocument')}
        title={t('writing.newDocument')}
        onclick={createDocument}
      >
        <ActionIcon name="file-plus" size={20} />
      </Button>
    </header>

    {#if snapshot.error}
      <Panel padding="md">
        <p class="writing__error" role="alert">{snapshot.error.message}</p>
      </Panel>
    {/if}

    {#if documents.length === 0}
      <Panel padding="lg">
        <p class="writing__notice">{t('writing.empty')}</p>
      </Panel>
    {:else}
      <ul class="writing__list">
        {#each documents as doc (doc.id)}
          <li class="writing__row">
            <button type="button" class="writing__card" onclick={() => open(doc.id)}>
              <span class="writing__card-title" use:tooltip={doc.title}>{doc.title}</span>
              <span class="writing__card-meta">{formatDate(doc.updated_at)}</span>
            </button>
            <IconButton
              class="writing__card-discard"
              size="sm"
              variant="ghost"
              label={t('writing.discard', { title: doc.title })}
              onclick={() => (pendingDiscard = doc)}
            >
              <ActionIcon name="delete" size={14} />
            </IconButton>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</section>

{#if pendingSectionDelete}
  <ConfirmDialog
    variant="destructive"
    title={t('writing.sectionDeleteTitle')}
    message={t('writing.sectionDeleteMessage', {
      title: pendingSectionDelete.title,
      words: String(pendingSectionWeight.words),
      headings: String(pendingSectionWeight.headings),
    })}
    confirmLabel={t('writing.sectionDeleteConfirm')}
    cancelLabel={t('writing.sectionDeleteCancel')}
    onconfirm={confirmDeleteSection}
    oncancel={() => (pendingSectionDelete = null)}
  />
{/if}

{#if pendingDiscard}
  <ConfirmDialog
    variant="destructive"
    title={t('writing.discardTitle')}
    message={t('writing.discardMessage', { title: pendingDiscard.title })}
    confirmLabel={t('writing.discardConfirm')}
    cancelLabel={t('writing.discardCancel')}
    onconfirm={confirmDiscard}
    oncancel={() => (pendingDiscard = null)}
  />
{/if}

<style>
  .writing {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-height: 0;
    height: 100%;
    padding: var(--space-5);
  }

  .writing__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
    flex-wrap: wrap;
  }

  .writing__eyebrow {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .writing__title {
    margin: var(--space-1) 0 0;
    font-family: var(--font-ui);
    font-size: var(--font-size-xl);
  }

  .writing__subtitle {
    margin: var(--space-1) 0 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .writing__bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  /* Reads as the heading it replaces until you put the caret in it. */
  .writing__doc-title {
    flex: 1;
    min-width: 0;
    min-height: 32px;
    margin: 0;
    padding: 0 var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-primary);
    font-family: var(--font-ui);
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    text-overflow: ellipsis;
  }

  .writing__doc-title:hover {
    border-color: var(--border-subtle);
  }

  .writing__doc-title:focus {
    outline: none;
    border-color: var(--border-focus);
    background: var(--color-surface-raised);
  }

  .writing__doc-title:focus-visible {
    box-shadow: var(--focus-ring);
  }

  .writing__bar-end {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .writing__revision {
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    font-variant-numeric: tabular-nums;
  }

  .writing__workspace {
    display: flex;
    flex: 1;
    min-height: 0;
    gap: var(--space-3);
  }

  .writing__editor {
    flex: 1;
    /* The floor is set inline from `EDITOR_MIN_WIDTH`, beside the bounds the
       resizer clamps to, so the two cannot drift apart. It is deliberately not
       zero: `min-width: 0` is how a flex child is normally told it may take the
       leftover room, but it also makes the manuscript the *first* thing
       flexbox squeezes, so the panels never reach the floor that would have
       made them give way. What that produced was a list of headings holding
       two thirds of the window while the prose wrapped one word per line. */
    min-height: 0;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    overflow: hidden;
  }

  /* Mirrors the outline's fixed column. With both folded away the editor
     panel takes the whole width; the text column inside it stays at its own
     measure, because a 200-character line is not a wider editor, it is an
     unreadable one. */
  .writing__research {
    display: flex;
    flex-direction: column;
    /* The basis is set inline from the persisted width, and the floor with it,
       from the same bounds module the resizer uses — so the numbers live in one
       place rather than in a stylesheet and a module that drift apart. The
       floor here is `squeeze`, not `min`: `min` is what a *drag* may not go
       past, and this is the window forcing the issue, where the alternative is
       not a narrower panel but the horizontal overflow §18 forbids.

       It shrinks but never grows: the spare room belongs to the manuscript. And
       it has to shrink, because the chosen width is the writer's and the window
       is not — dragged to its widest at a 900px window it would otherwise push
       the text column out of the viewport, which is the overflow §18 forbids.
       Yielding down to the floor is what makes "resizable" safe. */
    flex: 0 1 auto;
    min-height: 0;
    padding: var(--space-3) var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--surface-panel);
    overflow-y: auto;
  }

  .writing__outline {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    flex: 0 1 auto;
    min-height: 0;
    padding: var(--space-3) var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--surface-panel);
    overflow-y: auto;
  }

  /* The controls only appear on hover or keyboard focus. Five buttons beside
     every heading would turn the outline into a toolbar and bury the one thing
     it is for, which is reading the shape of the manuscript.

     They are laid over the row rather than beside it. An invisible button still
     takes its width, so in the flow they were stealing a third of the column
     from every title — the headings read as truncated at all times, with the
     space they needed sitting empty next to them. */
  .writing__outline-row {
    position: relative;
    display: flex;
    align-items: center;
  }

  .writing__outline-actions {
    position: absolute;
    inset-inline-end: 0;
    display: flex;
    align-items: center;
    gap: 2px;
    /* Opaque: it covers the end of a long title while it is showing, and a
       half-legible word under a row of icons is worse than a clean cut. */
    padding-inline-start: var(--space-3);
    background: var(--surface-panel);
    opacity: 0;
    pointer-events: none;
    transition: opacity var(--transition-base);
  }

  .writing__outline-row:hover .writing__outline-actions,
  .writing__outline-row:focus-within .writing__outline-actions {
    opacity: 1;
    pointer-events: auto;
  }

  .writing__outline-rename {
    width: 100%;
    min-height: 28px;
    padding: 0 var(--space-2);
    border: 1px solid var(--border-focus);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font: inherit;
    font-size: var(--font-size-xs);
  }

  .writing__outline-rename:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .writing__outline-title {
    margin: 0 0 0 var(--space-2);
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .writing__outline-empty {
    margin: 0 var(--space-2);
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .writing__outline-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .writing__outline-item {
    flex: 1;
    min-width: 0;
    display: block;
    width: 100%;
    min-height: 28px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-secondary);
    font: inherit;
    font-size: var(--font-size-sm);
    text-align: left;
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .writing__outline-item:hover {
    background: var(--color-surface-elevated);
    color: var(--color-text-primary);
  }

  .writing__outline-item:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .writing__notice,
  .writing__error {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .writing__error {
    color: var(--color-danger);
  }

  /* The document list is the Colecciones grid adapted to documents: the same
     track function and gap, with a 320px floor. The floor is chosen for what it
     yields rather than by analogy — across the Escritura panel it steps
     5 / 4 / 3 / 2 / 1 cards per row as the window narrows. A 280px floor fits
     six across, one more than a list meant to be read wants.

     `auto-fill`, not `auto-fit`: with two documents open, auto-fit would stretch
     each across half the screen. auto-fill keeps a card the size of a card. The
     two behave identically once there are more documents than columns.

     The track function measures the grid's OWN box, so the cards already follow
     the width of the Escritura panel — a container query would restate it. */
  .writing__list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: var(--space-3);
    align-content: start;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-y: auto;
  }

  .writing__source-notice {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .writing__source-title {
    margin: 0 0 var(--space-1);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-primary);
  }

  .writing__source-body {
    margin: 0;
    max-width: 68ch;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .writing__source-quote {
    margin: var(--space-2) 0 0;
    padding-left: var(--space-3);
    border-left: 2px solid var(--border-subtle);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-style: italic;
  }

  /* The message takes the room; the dismiss control keeps its own width at the
     end of the row and does not move as the count changes. */
  .writing__notice-row {
    display: flex;
    align-items: start;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .writing__notice-row .writing__notice {
    min-width: 0;
  }

  .writing__save-error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  /* The delete control stays a SIBLING of the card, never a child: a button
     nested in a button is invalid, and the browser would give the outer one the
     click either way. So the row is the positioning context and the control is
     laid over the corner the card reserves for it — it reads as part of the
     card while remaining an independent target that cannot open the document. */
  .writing__row {
    position: relative;
    display: flex;
    min-width: 0;
  }

  .writing__row :global(.writing__card-discard) {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
  }

  .writing__card {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    justify-content: space-between;
    gap: var(--space-2);
    flex: 1;
    min-width: 0;
    min-height: 44px;
    /* The right inset is the delete control's seat. Without it a long title
       runs under the button instead of ellipsing before it. sm is a 28px
       container, plus a gap on each side. */
    padding: var(--space-3) calc(var(--space-2) + 28px + var(--space-2)) var(--space-3)
      var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      background var(--transition-base),
      border-color var(--transition-base);
  }

  .writing__card:hover {
    background: var(--color-surface-elevated);
    border-color: var(--color-border-strong);
  }

  .writing__card:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  /* `align-items: stretch` above is what lets this ellipse: a column flex item
     that shrink-wraps its text has no width to overflow. */
  .writing__card-title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: var(--font-weight-medium);
  }

  .writing__card-meta {
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    font-variant-numeric: tabular-nums;
  }
</style>
