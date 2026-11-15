import { render, screen } from '@testing-library/svelte'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

import AudioPlayer from '../AudioPlayer.svelte'
import { stylesOf } from '../../__tests__/style-guards'

const styles = stylesOf(
  readFileSync(resolve(import.meta.dirname, '../AudioPlayer.svelte'), 'utf-8')
)

/** The declarations of the first rule whose selector contains `selector`. */
function ruleBody(selector: string): string {
  const start = styles.indexOf(selector)
  if (start < 0) return ''
  const open = styles.indexOf('{', start)
  const close = styles.indexOf('}', open)
  return styles.slice(open + 1, close)
}

describe('AudioPlayer design-system controls', () => {
  it('builds the skip buttons from the package Button', () => {
    render(AudioPlayer, { props: { src: '/audio/interview.wav' } })

    expect(screen.getByTestId('audio-skip-back')).toHaveClass('btn')
    expect(screen.getByTestId('audio-skip-forward')).toHaveClass('btn')
    expect(styles).not.toMatch(/\.audio-player__btn--skip/)
  })

  it('keeps play as the 56px hero target, drawn with the primary control tokens', () => {
    // OFF_SCALE_BY_DESIGN in apps/desktop/src/design-tokens.test.ts records this
    // size; IconButton's largest step is 40px.
    render(AudioPlayer, { props: { src: '/audio/interview.wav' } })
    expect(screen.getByTestId('audio-play-pause')).toHaveClass('audio-player__btn--play')

    const play = ruleBody('.audio-player__btn--play {')
    expect(play).toMatch(/width:\s*56px/)
    expect(play).toMatch(/height:\s*56px/)
    expect(play).toMatch(/background-color:\s*var\(--control-primary-bg\)/)
    expect(ruleBody('.audio-player__btn--play:focus-visible')).toMatch(/var\(--focus-ring\)/)
  })

  it('draws the volume slider track and thumb with tokens in every engine', () => {
    for (const pseudo of [
      '::-webkit-slider-runnable-track',
      '::-webkit-slider-thumb',
      '::-moz-range-track',
      '::-moz-range-thumb',
    ]) {
      const body = ruleBody(pseudo)
      expect(body, pseudo).toMatch(/var\(--/)
      expect(body, pseudo).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(/i)
    }
    expect(ruleBody('.audio-player__volume-slider {')).toMatch(/appearance:\s*none/)
  })

  it('keeps the volume slider a native range input', () => {
    render(AudioPlayer, { props: { src: '/audio/interview.wav' } })
    const slider = screen.getByTestId('audio-volume-slider')

    expect(slider.tagName).toBe('INPUT')
    expect(slider).toHaveAttribute('type', 'range')
  })

  it('animates the playback bar with a duration token', () => {
    expect(ruleBody('.audio-player__progress-fill')).toMatch(/transition:[^;]*var\(--duration-/)
    expect(styles).not.toMatch(/transition:[^;]*\b\d*\.?\d+m?s\b/)
  })
})
