import { render, screen, fireEvent } from '@testing-library/svelte'
import { describe, expect, it } from 'vitest'
import ZoteroTags from './ZoteroTags.svelte'

describe('ZoteroTags', () => {
  it('shows the manual tags and hides the automatic ones until asked', async () => {
    render(ZoteroTags, {
      props: { tags: ['metodología', 'tesis'], automaticTags: ['Historia social', '_toc'] },
    })
    expect(screen.getByText('metodología, tesis')).toBeInTheDocument()
    expect(screen.queryByText(/Historia social/)).not.toBeInTheDocument()

    const toggle = screen.getByRole('button', { name: 'Mostrar tags automáticos (2)' })
    expect(toggle).toHaveAttribute('aria-expanded', 'false')
    await fireEvent.click(toggle)
    expect(screen.getByText('Historia social, _toc')).toBeInTheDocument()
    const hide = screen.getByRole('button', { name: 'Ocultar tags automáticos' })
    expect(hide).toHaveAttribute('aria-expanded', 'true')

    await fireEvent.click(hide)
    expect(screen.queryByText(/Historia social/)).not.toBeInTheDocument()
  })

  it('offers no toggle when the work has no automatic tags', () => {
    render(ZoteroTags, { props: { tags: ['tesis'], automaticTags: [] } })
    expect(screen.getByText('tesis')).toBeInTheDocument()
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })

  it('keeps the toggle when every tag is automatic', async () => {
    render(ZoteroTags, { props: { tags: [], automaticTags: ['_toc'] } })
    await fireEvent.click(screen.getByRole('button', { name: 'Mostrar tags automáticos (1)' }))
    expect(screen.getByText('_toc')).toBeInTheDocument()
  })
})
