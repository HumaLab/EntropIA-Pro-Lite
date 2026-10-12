import { describe, expect, it } from 'vitest'
import { existsSync, readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../../../..')

function readRepoFile(relative: string): string {
  return readFileSync(resolve(repoRoot, relative), 'utf8')
}

/** Top-level job body: its `  <name>:` key up to the next job key. */
function jobText(workflow: string, job: string): string {
  const start = workflow.indexOf(`\n  ${job}:\n`)
  expect(start, `job ${job} not found`).toBeGreaterThanOrEqual(0)
  const body = workflow.slice(start + 1)
  const next = body.slice(1).search(/\n {2}[a-z][a-z0-9_-]*:\n/)
  return next === -1 ? body : body.slice(0, next + 1)
}

/** `- package-ecosystem: <name>` blocks of dependabot.yml, keyed by ecosystem. */
function dependabotUpdates(config: string): Map<string, string> {
  const blocks = config.split(/\n(?= {2}- package-ecosystem:)/).slice(1)
  return new Map(
    blocks.map((block) => {
      const ecosystem = /package-ecosystem:\s*['"]?([\w-]+)/.exec(block)?.[1] ?? ''
      return [ecosystem, block]
    })
  )
}

describe('CI supply chain (C-03)', () => {
  it('keeps npm, cargo and GitHub Actions up to date with grouped minor updates', () => {
    expect(existsSync(resolve(repoRoot, '.github/dependabot.yml'))).toBe(true)
    const updates = dependabotUpdates(readRepoFile('.github/dependabot.yml'))

    expect([...updates.keys()].sort()).toEqual(['cargo', 'github-actions', 'npm'])
    expect(updates.get('npm')).toMatch(/directory:\s*['"]?\/['"]?\n/)
    expect(updates.get('cargo')).toMatch(/directory:\s*['"]?\/apps\/desktop\/src-tauri['"]?\n/)
    expect(updates.get('github-actions')).toMatch(/directory:\s*['"]?\/['"]?\n/)

    for (const [ecosystem, block] of updates) {
      expect(block, ecosystem).toMatch(/update-types:[\s\S]*minor[\s\S]*patch/)
    }
  })

  it('blocks CI on high production npm advisories and on unignored cargo advisories', () => {
    const audit = jobText(readRepoFile('.github/workflows/ci.yml'), 'audit')
    // Phase 2 closed the known advisories, so the job gates from day one.
    expect(audit).not.toMatch(/continue-on-error:\s*true/)
    expect(audit).toContain('pnpm audit --prod --audit-level high')
    expect(audit).toMatch(/cargo audit\b/)
    expect(audit).not.toMatch(/\|\|\s*true/)
  })
})

describe('engine pin bump goes through a pull request (D-05)', () => {
  const workflow = readRepoFile('.github/workflows/engine-pin-bump.yml')

  it('never pushes to main directly', () => {
    expect(workflow).not.toMatch(/refs\/heads\/main/)
    expect(workflow).not.toMatch(/git push[^\n]*\bmain\b/)
  })

  it('opens or reuses a pull request for the tested bump', () => {
    expect(workflow).toContain('gh pr create')
    expect(workflow).toMatch(/gh pr list[^\n]*--head "\$BUMP_BRANCH"/)
  })

  it('refreshes title and body together when the PR already exists', () => {
    // The body names the pin and the tested SHA; a reused PR must not keep the old ones.
    expect(workflow).toMatch(/gh pr edit "\$existing" --title "\$title" --body "\$body"/)
  })

  it('keeps the explicit CI dispatch, since GITHUB_TOKEN events start no workflows', () => {
    expect(workflow).toContain('gh workflow run ci.yml --ref "$BUMP_BRANCH"')
    const permissions = /\npermissions:\n((?: {2}.*\n)+)/.exec(workflow)?.[1] ?? ''
    expect(permissions).toMatch(/contents: write/)
    expect(permissions).toMatch(/pull-requests: write/)
    expect(permissions).toMatch(/actions: write/)
  })
})
