import { render, screen, fireEvent, waitFor } from '@testing-library/svelte'
import { describe, it, expect, vi } from 'vitest'
import EntityViewer from '../EntityViewer.svelte'
import entityViewerSource from '../EntityViewer.svelte?raw'
import type { Entity, EntityType, EntityViewerProps } from '../EntityViewer.types'

const makeEntity = (overrides: Partial<Entity> = {}): Entity => ({
  id: 'ent-1',
  itemId: 'item-1',
  entityType: 'person',
  value: 'Don Manuel Belgrano',
  startOffset: 0,
  endOffset: 19,
  confidence: 1.0,
  createdAt: 1700000000000,
  ...overrides,
})

describe('EntityViewer', () => {
  // ─────────────────────────────────────────────────────────────────────────
  // Empty state
  // ─────────────────────────────────────────────────────────────────────────

  it('shows empty state message when entities array is empty', () => {
    render(EntityViewer, { props: { entities: [] } })
    expect(screen.getByTestId('entity-viewer-empty')).toBeInTheDocument()
  })

  it('does not render any chip when entities is empty', () => {
    render(EntityViewer, { props: { entities: [] } })
    expect(screen.queryAllByRole('group')).toHaveLength(0)
  })

  // ─────────────────────────────────────────────────────────────────────────
  // Flat rendering
  // ─────────────────────────────────────────────────────────────────────────

  it('renders one chip per entity without grouping them', () => {
    const entities: Entity[] = [
      makeEntity({ id: 'e1', entityType: 'person', value: 'Don Manuel Belgrano' }),
      makeEntity({ id: 'e2', entityType: 'person', value: 'Doña Juana Azurduy' }),
      makeEntity({ id: 'e3', entityType: 'place', value: 'ciudad de Buenos Aires' }),
      makeEntity({ id: 'e4', entityType: 'date', value: '15 de mayo de 1810' }),
    ]
    render(EntityViewer, { props: { entities } })
    expect(screen.getAllByRole('group')).toHaveLength(4)
  })

  it('does not render type headings, keeping the type on the chip itself', () => {
    const entities: Entity[] = [
      makeEntity({ id: 'e1', entityType: 'person', value: 'Fray Bartolomé' }),
      makeEntity({ id: 'e2', entityType: 'institution', value: 'Cabildo' }),
    ]
    render(EntityViewer, { props: { entities } })

    expect(screen.queryByText(/^person$/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/^institution$/i)).not.toBeInTheDocument()

    expect(screen.getByTestId('entity-chip-e1')).toHaveClass('entity-viewer__chip--person')
    expect(screen.getByTestId('entity-chip-e2')).toHaveClass('entity-viewer__chip--institution')
  })

  it('orders chips by entity type so same-type colors stay adjacent', () => {
    const entities: Entity[] = [
      makeEntity({ id: 'd1', entityType: 'date', value: '1810' }),
      makeEntity({ id: 'p1', entityType: 'person', value: 'Belgrano' }),
      makeEntity({ id: 'd2', entityType: 'date', value: '1816' }),
      makeEntity({ id: 'p2', entityType: 'person', value: 'Azurduy' }),
    ]
    render(EntityViewer, { props: { entities } })

    const ids = screen
      .getAllByRole('group')
      .map((chip) => chip.getAttribute('data-testid')?.replace('entity-chip-', ''))

    expect(ids).toEqual(['p1', 'p2', 'd1', 'd2'])
  })

  it('renders all entity values as pills', () => {
    const entities: Entity[] = [
      makeEntity({ id: 'e1', entityType: 'person', value: 'Don Manuel Belgrano' }),
      makeEntity({ id: 'e2', entityType: 'place', value: 'ciudad de Buenos Aires' }),
    ]
    render(EntityViewer, { props: { entities } })
    expect(screen.getByText('Don Manuel Belgrano')).toBeInTheDocument()
    expect(screen.getByText('ciudad de Buenos Aires')).toBeInTheDocument()
  })

  it('color-codes one chip per type when all types are present', () => {
    const entities: Entity[] = [
      makeEntity({ id: 'e1', entityType: 'person', value: 'Don Pedro' }),
      makeEntity({ id: 'e2', entityType: 'place', value: 'villa de Potosí' }),
      makeEntity({ id: 'e3', entityType: 'date', value: '12 de octubre de 1492' }),
      makeEntity({ id: 'e4', entityType: 'institution', value: 'Real Audiencia' }),
    ]
    render(EntityViewer, { props: { entities } })

    expect(screen.getAllByRole('group')).toHaveLength(4)
    expect(screen.getByTestId('entity-chip-e2')).toHaveClass('entity-viewer__chip--place')
    expect(screen.getByTestId('entity-chip-e3')).toHaveClass('entity-viewer__chip--date')
  })

  it('renders PLACE entities as place-colored chips', () => {
    const entities: Entity[] = [
      makeEntity({ id: 'p1', entityType: 'place', value: 'río de la Plata' }),
      makeEntity({ id: 'p2', entityType: 'place', value: 'ciudad de Córdoba' }),
    ]
    render(EntityViewer, { props: { entities } })
    expect(screen.getByText('río de la Plata')).toBeInTheDocument()
    expect(screen.getByText('ciudad de Córdoba')).toBeInTheDocument()
    expect(screen.getByTestId('entity-chip-p1')).toHaveClass('entity-viewer__chip--place')
  })

  it('renders DATE entities as date-colored chips', () => {
    const entities: Entity[] = [makeEntity({ id: 'd1', entityType: 'date', value: '25/05/1810' })]
    render(EntityViewer, { props: { entities } })
    expect(screen.getByText('25/05/1810')).toBeInTheDocument()
    expect(screen.getByTestId('entity-chip-d1')).toHaveClass('entity-viewer__chip--date')
  })

  it('renders INSTITUTION entities as institution-colored chips', () => {
    const entities: Entity[] = [
      makeEntity({ id: 'i1', entityType: 'institution', value: 'Real Audiencia' }),
    ]
    render(EntityViewer, { props: { entities } })
    expect(screen.getByText('Real Audiencia')).toBeInTheDocument()
    expect(screen.getByTestId('entity-chip-i1')).toHaveClass('entity-viewer__chip--institution')
  })

  it('shows NER tag on entity pill instead of confidence percentage', () => {
    const entity = makeEntity({
      id: 'c1',
      entityType: 'person',
      value: 'Don Manuel',
      confidence: 0.85,
    })
    render(EntityViewer, { props: { entities: [entity] } })
    expect(screen.getByText('PER')).toBeInTheDocument()
    expect(screen.queryByTestId('entity-confidence')).not.toBeInTheDocument()
  })

  it('renders organization entities with their own chip color and tag', () => {
    const entity = makeEntity({
      id: 'c2',
      entityType: 'organization',
      value: 'Wilson Sons y Cía.',
      confidence: 1.0,
    })
    render(EntityViewer, { props: { entities: [entity] } })
    expect(screen.queryByText(/^organization$/i)).not.toBeInTheDocument()
    expect(screen.getByTestId('entity-chip-c2')).toHaveClass('entity-viewer__chip--organization')
    expect(screen.getByText('ORG')).toBeInTheDocument()
  })

  // ─────────────────────────────────────────────────────────────────────────
  // Click dispatches highlight event
  // ─────────────────────────────────────────────────────────────────────────

  it('clicking an entity pill calls onhighlight with correct offsets', async () => {
    const onhighlight = vi.fn()
    const entity = makeEntity({ startOffset: 10, endOffset: 30, value: 'Don Manuel Belgrano' })
    render(EntityViewer, { props: { entities: [entity], onhighlight } })

    const pill = screen.getByText('Don Manuel Belgrano')
    await fireEvent.click(pill)

    expect(onhighlight).toHaveBeenCalledOnce()
    expect(onhighlight).toHaveBeenCalledWith({ startOffset: 10, endOffset: 30 })
  })

  it('does not call onhighlight when entity has null offsets', async () => {
    const onhighlight = vi.fn()
    const entity = makeEntity({ startOffset: null, endOffset: null, value: 'Sin offset' })
    render(EntityViewer, { props: { entities: [entity], onhighlight } })

    const pill = screen.getByText('Sin offset')
    await fireEvent.click(pill)

    expect(onhighlight).not.toHaveBeenCalled()
  })

  it('clicking an entity pill calls onentityclick with the entity', async () => {
    const onentityclick = vi.fn()
    const entity = makeEntity({ id: 'e-click', value: 'Mar del Plata' })
    render(EntityViewer, { props: { entities: [entity], onentityclick } })

    await fireEvent.click(screen.getByText('Mar del Plata'))

    expect(onentityclick).toHaveBeenCalledOnce()
    expect(onentityclick).toHaveBeenCalledWith(expect.objectContaining({ id: 'e-click' }))
  })

  it('enters inline edit mode when clicking a chip', async () => {
    const props: EntityViewerProps = {
      entities: [makeEntity({ id: 'entity-inline', value: 'Mar del Plata' })],
      editingEntityId: null,
      editingValue: '',
    }

    const view = render(EntityViewer, { props })

    props.onentityclick = async (entity: Entity) => {
      props.editingEntityId = entity.id
      props.editingValue = entity.value
      await view.rerender(props)
    }

    await view.rerender(props)
    await fireEvent.click(screen.getByRole('button', { name: /Mar del Plata/i }))

    expect(await screen.findByDisplayValue('Mar del Plata')).toBeInTheDocument()
  })

  it('shows the current entity value inside the inline input', () => {
    render(EntityViewer, {
      props: {
        entities: [makeEntity({ id: 'entity-current', value: 'Belgrano' })],
        editingEntityId: 'entity-current',
        editingValue: 'Belgrano',
      },
    })

    expect(screen.getByRole('textbox', { name: 'Edit entity value' })).toHaveValue('Belgrano')
  })

  it('pressing Enter triggers save callback with trimmed value', async () => {
    const onsaveentity = vi.fn()
    const oneditvaluechange = vi.fn()

    render(EntityViewer, {
      props: {
        entities: [makeEntity({ id: 'entity-save', value: 'Belgrano' })],
        editingEntityId: 'entity-save',
        editingValue: '  Belgrano renovado  ',
        onsaveentity,
        oneditvaluechange,
      },
    })

    const input = screen.getByRole('textbox', { name: 'Edit entity value' })
    await fireEvent.keyDown(input, { key: 'Enter' })

    expect(onsaveentity).toHaveBeenCalledWith('entity-save', 'Belgrano renovado')
    expect(oneditvaluechange).not.toHaveBeenCalled()
  })

  it('pressing Escape cancels inline editing', async () => {
    const oncancelentityedit = vi.fn()

    render(EntityViewer, {
      props: {
        entities: [makeEntity({ id: 'entity-cancel', value: 'Belgrano' })],
        editingEntityId: 'entity-cancel',
        editingValue: 'Belgrano editado',
        oncancelentityedit,
      },
    })

    await fireEvent.keyDown(screen.getByRole('textbox', { name: 'Edit entity value' }), {
      key: 'Escape',
    })

    expect(oncancelentityedit).toHaveBeenCalledOnce()
  })

  it('requires inline delete confirmation before triggering delete callback', async () => {
    const ondeleteentity = vi.fn()

    render(EntityViewer, {
      props: {
        entities: [makeEntity({ id: 'entity-delete', value: 'Belgrano' })],
        ondeleteentity,
      },
    })

    expect(screen.queryByRole('button', { name: 'Delete entity Belgrano' })).not.toBeInTheDocument()

    await fireEvent.mouseEnter(screen.getByTestId('entity-chip-entity-delete'))

    const deleteButton = await screen.findByRole('button', { name: 'Delete entity Belgrano' })
    await fireEvent.click(deleteButton)

    expect(ondeleteentity).not.toHaveBeenCalled()

    const confirmButton = await screen.findByRole('button', {
      name: 'Confirm delete entity Belgrano',
    })
    expect(confirmButton).toHaveTextContent('Delete?')
    expect(confirmButton).toHaveAttribute('title', 'Press again to confirm delete')
    await fireEvent.click(confirmButton)

    expect(ondeleteentity).toHaveBeenCalledWith('entity-delete')
  })

  it('supports keyboard-first delete confirmation without breaking inline edit affordances', async () => {
    const ondeleteentity = vi.fn()

    render(EntityViewer, {
      props: {
        entities: [makeEntity({ id: 'entity-delete-keyboard', value: 'Belgrano' })],
        ondeleteentity,
      },
    })

    const pill = screen.getByRole('button', { name: /Belgrano/i })

    await fireEvent.focusIn(pill)

    const deleteButton = await screen.findByRole('button', { name: 'Delete entity Belgrano' })
    await fireEvent.keyDown(deleteButton, { key: 'Enter' })

    expect(ondeleteentity).not.toHaveBeenCalled()
    const confirmButton = await screen.findByRole('button', {
      name: 'Confirm delete entity Belgrano',
    })
    expect(confirmButton).toHaveTextContent('Delete?')

    await fireEvent.keyDown(confirmButton, { key: 'Enter' })

    expect(ondeleteentity).toHaveBeenCalledWith('entity-delete-keyboard')
  })

  it('paints every chip background opaque so the inline controls never blend into what is behind', () => {
    // Los controles flotan sobre el borde del chip y su degradado reusa este
    // fondo: con un token translúcido se veía el texto de atrás a través de
    // ambos. `--color-accent-faint` (6% de alfa) rompía justo el chip ORG.
    const backgrounds = Array.from(
      entityViewerSource.matchAll(/--entity-chip-bg:\s*([^;]+);/g),
      ([, value]) => (value ?? '').trim()
    )

    expect(backgrounds.length).toBeGreaterThanOrEqual(7)

    for (const background of backgrounds) {
      expect(background).not.toMatch(/-faint\)/)
      expect(background).not.toMatch(/(rgba|hsla)\(/)
      expect(background).not.toMatch(/transparent/)
    }
  })

  it('exposes an edit icon on hover that opens inline editing, like a triple row', async () => {
    const onentityclick = vi.fn()

    render(EntityViewer, {
      props: {
        entities: [makeEntity({ id: 'entity-pencil', value: 'Belgrano' })],
        onentityclick,
      },
    })

    expect(screen.queryByRole('button', { name: 'Edit entity Belgrano' })).not.toBeInTheDocument()

    await fireEvent.mouseEnter(screen.getByTestId('entity-chip-entity-pencil'))

    const editButton = await screen.findByRole('button', { name: 'Edit entity Belgrano' })
    expect(editButton).toHaveAttribute('title', 'Edit entity')
    await fireEvent.click(editButton)

    expect(onentityclick).toHaveBeenCalledWith(expect.objectContaining({ id: 'entity-pencil' }))
  })

  it('offers save and cancel icons while editing instead of relying on blur alone', async () => {
    const onsaveentity = vi.fn()
    const oncancelentityedit = vi.fn()

    render(EntityViewer, {
      props: {
        entities: [makeEntity({ id: 'entity-icons', value: 'Belgrano' })],
        editingEntityId: 'entity-icons',
        editingValue: '  Belgrano renovado  ',
        onsaveentity,
        oncancelentityedit,
      },
    })

    await fireEvent.click(screen.getByTestId('entity-save-entity-icons'))
    expect(onsaveentity).toHaveBeenCalledWith('entity-icons', 'Belgrano renovado')

    await fireEvent.click(screen.getByTestId('entity-cancel-entity-icons'))
    expect(oncancelentityedit).toHaveBeenCalledOnce()
  })

  it('disables the save icon while the edited value is empty', () => {
    render(EntityViewer, {
      props: {
        entities: [makeEntity({ id: 'entity-empty', value: 'Belgrano' })],
        editingEntityId: 'entity-empty',
        editingValue: '   ',
      },
    })

    expect(screen.getByTestId('entity-save-entity-empty')).toBeDisabled()
  })

  // ──────────────────────────────────────────────────────────────────────────
  // Manual creation through the trailing add chip
  // ──────────────────────────────────────────────────────────────────────────

  const creatable: EntityType[] = ['person', 'organization', 'place']

  it('renders no add chip unless creatable types are offered', () => {
    render(EntityViewer, { props: { entities: [makeEntity({ id: 'e1' })] } })
    expect(screen.queryByTestId('entity-add')).not.toBeInTheDocument()
  })

  it('renders the add chip as the last chip of the list', () => {
    const entities: Entity[] = [
      makeEntity({ id: 'e1', entityType: 'person', value: 'Belgrano' }),
      makeEntity({ id: 'e2', entityType: 'date', value: '1810' }),
    ]
    render(EntityViewer, { props: { entities, creatableTypes: creatable } })

    const wrap = screen.getByTestId('entity-add').parentElement
    const testIds = Array.from(wrap?.children ?? []).map((child) =>
      child.getAttribute('data-testid')
    )

    expect(testIds).toEqual(['entity-chip-e1', 'entity-chip-e2', 'entity-add'])
  })

  it('shows the add chip alone, with no empty state, when nothing was extracted', () => {
    render(EntityViewer, { props: { entities: [], creatableTypes: creatable } })

    expect(screen.queryByTestId('entity-viewer-empty')).not.toBeInTheDocument()
    const wrap = screen.getByTestId('entity-add').parentElement
    expect(wrap?.children).toHaveLength(1)
  })

  it('keeps the type selector and the value field out of sight until the chip is clicked', async () => {
    render(EntityViewer, { props: { entities: [], creatableTypes: creatable } })

    expect(screen.queryByRole('textbox', { name: 'New entity value' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'New entity type' })).not.toBeInTheDocument()

    await fireEvent.click(screen.getByTestId('entity-add'))

    expect(screen.getByRole('textbox', { name: 'New entity value' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'New entity type' })).toBeInTheDocument()
    // El chip de alta cede su lugar al borrador: no quedan los dos.
    expect(screen.queryByTestId('entity-add')).not.toBeInTheDocument()
  })

  it('offers only the creatable types, labelled by their NER tag', async () => {
    render(EntityViewer, { props: { entities: [], creatableTypes: creatable } })
    await fireEvent.click(screen.getByTestId('entity-add'))
    await fireEvent.click(screen.getByTestId('entity-new-type'))

    const options = screen.getAllByRole('option').map((option) => option.textContent?.trim())

    expect(options).toEqual(['PER', 'ORG', 'LOC'])
  })

  it('picks the tag from a listbox of its own instead of a native select', async () => {
    const { container } = render(EntityViewer, {
      props: { entities: [], creatableTypes: creatable },
    })
    await fireEvent.click(screen.getByTestId('entity-add'))

    // Un <select> nativo lo dibuja el sistema operativo: ni el fondo ni la
    // tipografía del popup llegan desde el CSS de la app.
    expect(container.querySelector('select')).toBeNull()

    const trigger = screen.getByTestId('entity-new-type')
    expect(trigger).toHaveAttribute('aria-haspopup', 'listbox')
    expect(trigger).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()

    await fireEvent.click(trigger)

    expect(trigger).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByRole('listbox')).toBeInTheDocument()
    // Sin tipo explícito el borrador toma el primero ofrecido.
    expect(screen.getByRole('option', { name: /PER/ })).toHaveAttribute('aria-selected', 'true')
    expect(screen.getByRole('option', { name: /ORG/ })).toHaveAttribute('aria-selected', 'false')
  })

  it('walks the tag list with the arrow keys and commits on Enter', async () => {
    const onnewentitytypechange = vi.fn()
    render(EntityViewer, {
      props: { entities: [], creatableTypes: creatable, onnewentitytypechange },
    })

    await fireEvent.click(screen.getByTestId('entity-add'))
    await fireEvent.keyDown(screen.getByTestId('entity-new-type'), { key: 'ArrowDown' })

    const listbox = screen.getByRole('listbox')
    // Abre parado sobre el tipo vigente, que por defecto es el primero.
    expect(listbox).toHaveAttribute('aria-activedescendant', 'entity-type-option-person')

    await fireEvent.keyDown(listbox, { key: 'ArrowDown' })
    expect(listbox).toHaveAttribute('aria-activedescendant', 'entity-type-option-organization')

    await fireEvent.keyDown(listbox, { key: 'Enter' })

    expect(onnewentitytypechange).toHaveBeenCalledWith('organization')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
  })

  it('closes the tag list on Escape without discarding the draft', async () => {
    render(EntityViewer, {
      props: { entities: [], creatableTypes: creatable, newEntityValue: 'Juana Rouco' },
    })

    await fireEvent.click(screen.getByTestId('entity-add'))
    await fireEvent.click(screen.getByTestId('entity-new-type'))
    await fireEvent.keyDown(screen.getByRole('listbox'), { key: 'Escape' })

    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(screen.getByTestId('entity-new-chip')).toBeInTheDocument()
  })

  it('creates on Enter and closes the draft, but never mid-IME composition', async () => {
    const oncreateentity = vi.fn().mockResolvedValue(true)
    render(EntityViewer, {
      props: {
        entities: [],
        creatableTypes: creatable,
        newEntityValue: 'Juana Rouco',
        oncreateentity,
      },
    })

    await fireEvent.click(screen.getByTestId('entity-add'))
    const input = screen.getByRole('textbox', { name: 'New entity value' })

    await fireEvent.keyDown(input, { key: 'Enter', isComposing: true })
    expect(oncreateentity).not.toHaveBeenCalled()

    await fireEvent.keyDown(input, { key: 'Enter' })
    expect(oncreateentity).toHaveBeenCalledTimes(1)

    await waitFor(() => {
      expect(screen.getByTestId('entity-add')).toBeInTheDocument()
    })
  })

  it('keeps the draft open with what was typed when creation fails', async () => {
    const oncreateentity = vi.fn().mockResolvedValue(false)
    render(EntityViewer, {
      props: {
        entities: [],
        creatableTypes: creatable,
        newEntityValue: 'Juana Rouco',
        oncreateentity,
      },
    })

    await fireEvent.click(screen.getByTestId('entity-add'))
    await fireEvent.click(screen.getByTestId('entity-new-save'))

    expect(oncreateentity).toHaveBeenCalledTimes(1)
    expect(screen.getByTestId('entity-new-chip')).toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: 'New entity value' })).toHaveValue('Juana Rouco')
  })

  it('disables the save icon and refuses Enter while the draft value is empty', async () => {
    const oncreateentity = vi.fn().mockResolvedValue(true)
    render(EntityViewer, {
      props: { entities: [], creatableTypes: creatable, newEntityValue: '  ', oncreateentity },
    })

    await fireEvent.click(screen.getByTestId('entity-add'))

    expect(screen.getByTestId('entity-new-save')).toBeDisabled()
    await fireEvent.keyDown(screen.getByRole('textbox', { name: 'New entity value' }), {
      key: 'Enter',
    })
    expect(oncreateentity).not.toHaveBeenCalled()
  })

  it('discards the draft on Escape and on the cancel icon, clearing what was typed', async () => {
    const onnewentityvaluechange = vi.fn()
    render(EntityViewer, {
      props: {
        entities: [],
        creatableTypes: creatable,
        newEntityValue: 'Juana Rouco',
        onnewentityvaluechange,
      },
    })

    await fireEvent.click(screen.getByTestId('entity-add'))
    await fireEvent.keyDown(screen.getByRole('textbox', { name: 'New entity value' }), {
      key: 'Escape',
    })

    expect(screen.getByTestId('entity-add')).toBeInTheDocument()
    expect(onnewentityvaluechange).toHaveBeenLastCalledWith('')

    await fireEvent.click(screen.getByTestId('entity-add'))
    await fireEvent.click(screen.getByTestId('entity-new-cancel'))

    expect(screen.getByTestId('entity-add')).toBeInTheDocument()
    expect(onnewentityvaluechange).toHaveBeenLastCalledWith('')
  })

  it('reports the chosen type upward and tints the draft chip with it', async () => {
    const onnewentitytypechange = vi.fn()
    render(EntityViewer, {
      props: {
        entities: [],
        creatableTypes: creatable,
        newEntityType: 'place' as EntityType,
        onnewentitytypechange,
      },
    })

    await fireEvent.click(screen.getByTestId('entity-add'))

    expect(screen.getByTestId('entity-new-chip')).toHaveClass('entity-viewer__chip--place')

    await fireEvent.click(screen.getByTestId('entity-new-type'))
    await fireEvent.click(screen.getByTestId('entity-new-type-organization'))
    expect(onnewentitytypechange).toHaveBeenCalledWith('organization')
  })

  it('carries the add action as both label and tooltip on an icon-only chip', () => {
    render(EntityViewer, { props: { entities: [], creatableTypes: creatable } })

    const chip = screen.getByTestId('entity-add')
    expect(chip.textContent?.trim()).toBe('')
    expect(chip.querySelector('svg')).not.toBeNull()
    expect(chip.getAttribute('title')).toBe(chip.getAttribute('aria-label'))
    expect(chip).toHaveAccessibleName('Add entity')
  })

  it('blur saves changed non-empty values and cancels unchanged ones', async () => {
    const onsaveentity = vi.fn()
    const oncancelentityedit = vi.fn()
    const props: EntityViewerProps = {
      entities: [makeEntity({ id: 'entity-blur', value: 'Belgrano' })],
      editingEntityId: 'entity-blur',
      editingValue: '  Belgrano actualizado  ',
      onsaveentity,
      oncancelentityedit,
    }

    const view = render(EntityViewer, { props })
    const input = screen.getByRole('textbox', { name: 'Edit entity value' })

    await fireEvent.blur(input)

    expect(onsaveentity).toHaveBeenCalledWith('entity-blur', 'Belgrano actualizado')
    expect(oncancelentityedit).not.toHaveBeenCalled()

    props.editingValue = '   '
    onsaveentity.mockClear()
    await view.rerender(props)
    await fireEvent.blur(screen.getByRole('textbox', { name: 'Edit entity value' }))

    await waitFor(() => {
      expect(oncancelentityedit).toHaveBeenCalledOnce()
    })
    expect(onsaveentity).not.toHaveBeenCalled()
  })
})
