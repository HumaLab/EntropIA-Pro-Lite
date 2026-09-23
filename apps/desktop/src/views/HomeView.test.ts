/** @vitest-environment jsdom */

import { cleanup, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it } from 'vitest'
import { locale } from '$lib/i18n'
import HomeView from './HomeView.svelte'

afterEach(() => {
  cleanup()
  locale.set('es')
})

describe('HomeView', () => {
  it('renders the startup page shell with the Inicio header', () => {
    const { container } = render(HomeView)

    const root = container.querySelector('.home-view')
    expect(root).not.toBeNull()
    expect(root).toHaveClass('page-shell')

    expect(screen.getByRole('heading', { level: 1, name: 'Inicio' })).toBeInTheDocument()
    expect(screen.getByText('Espacio de trabajo')).toBeInTheDocument()
  })

  it('renders the English header when the locale is English', () => {
    locale.set('en')
    render(HomeView)

    expect(screen.getByRole('heading', { level: 1, name: 'Home' })).toBeInTheDocument()
    expect(screen.getByText('Workspace')).toBeInTheDocument()
  })
})
