import { render, screen } from '@testing-library/svelte'
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import ProgressBar from '../ProgressBar.svelte'

const source = readFileSync(resolve(import.meta.dirname, '../ProgressBar.svelte'), 'utf-8')

const fillOf = (bar: HTMLElement) => bar.querySelector<HTMLElement>('.progress-bar__fill')!

describe('ProgressBar', () => {
  it('exposes a progressbar with its value range and name', () => {
    render(ProgressBar, { props: { value: 40, label: 'Downloading model' } })

    const bar = screen.getByRole('progressbar', { name: 'Downloading model' })
    expect(bar).toHaveAttribute('aria-valuemin', '0')
    expect(bar).toHaveAttribute('aria-valuemax', '100')
    expect(bar).toHaveAttribute('aria-valuenow', '40')
    expect(fillOf(bar).style.width).toBe('40%')
  })

  it('scales the fill to a custom max', () => {
    render(ProgressBar, { props: { value: 3, max: 4, label: 'Files' } })

    const bar = screen.getByRole('progressbar', { name: 'Files' })
    expect(bar).toHaveAttribute('aria-valuemax', '4')
    expect(bar).toHaveAttribute('aria-valuenow', '3')
    expect(fillOf(bar).style.width).toBe('75%')
  })

  it('clamps values outside 0..max', () => {
    const { rerender } = render(ProgressBar, { props: { value: 140, label: 'Over' } })
    const bar = screen.getByRole('progressbar', { name: 'Over' })
    expect(bar).toHaveAttribute('aria-valuenow', '100')
    expect(fillOf(bar).style.width).toBe('100%')

    return rerender({ value: -5 }).then(() => {
      expect(bar).toHaveAttribute('aria-valuenow', '0')
      expect(fillOf(bar).style.width).toBe('0%')
    })
  })

  it('treats a non-finite value as 0', () => {
    render(ProgressBar, { props: { value: Number.NaN, label: 'Unknown' } })
    const bar = screen.getByRole('progressbar', { name: 'Unknown' })
    expect(bar).toHaveAttribute('aria-valuenow', '0')
  })

  it('passes a value text through', () => {
    render(ProgressBar, { props: { value: 2, max: 5, label: 'Steps', valueText: '2 of 5' } })
    const bar = screen.getByRole('progressbar', { name: 'Steps' })
    expect(bar).toHaveAttribute('aria-valuetext', '2 of 5')
  })

  it('omits aria-valuenow while indeterminate', () => {
    render(ProgressBar, { props: { value: 30, indeterminate: true, label: 'Indexing' } })

    const bar = screen.getByRole('progressbar', { name: 'Indexing' })
    expect(bar).not.toHaveAttribute('aria-valuenow')
    expect(bar).not.toHaveAttribute('aria-valuetext')
    expect(bar).toHaveClass('progress-bar--indeterminate')
  })

  it('takes its name from aria-labelledby', () => {
    const heading = document.createElement('span')
    heading.id = 'progress-heading'
    heading.textContent = 'Reranker'
    document.body.appendChild(heading)

    render(ProgressBar, { props: { value: 10, 'aria-labelledby': 'progress-heading' } })
    expect(screen.getByRole('progressbar', { name: 'Reranker' })).toBeInTheDocument()
    heading.remove()
  })

  it('defaults to the accent tone and accepts the status tones', () => {
    const { rerender } = render(ProgressBar, { props: { value: 50, label: 'Tone' } })
    const bar = screen.getByRole('progressbar', { name: 'Tone' })
    expect(bar).toHaveClass('progress-bar--accent')

    return (async () => {
      for (const tone of ['success', 'warning', 'danger'] as const) {
        await rerender({ tone })
        expect(bar).toHaveClass(`progress-bar--${tone}`)
        expect(bar).not.toHaveClass('progress-bar--accent')
      }
    })()
  })

  it('maps each tone to its colour token', () => {
    for (const tone of ['accent', 'success', 'warning', 'danger']) {
      expect(source).toMatch(
        new RegExp(String.raw`\.progress-bar--${tone}\s*\{[^}]*var\(--color-${tone}\)`)
      )
    }
  })

  it('applies the size modifier and extra classes', () => {
    render(ProgressBar, { props: { value: 5, size: 'sm', label: 'Small', class: 'extra' } })
    const bar = screen.getByRole('progressbar', { name: 'Small' })
    expect(bar).toHaveClass('progress-bar--sm')
    expect(bar).toHaveClass('extra')
  })

  it('stops its motion under prefers-reduced-motion', () => {
    expect(source).toMatch(/@media \(prefers-reduced-motion: reduce\)\s*\{[\s\S]*animation: none/)
    expect(source).toMatch(/transition: width var\(--transition-smooth\)/)
  })
})
