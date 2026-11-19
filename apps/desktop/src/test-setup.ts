import '@testing-library/jest-dom'
import { afterEach, vi } from 'vitest'

// Mock Tauri APIs globally — tests run in happy-dom, not Tauri
vi.mock('@tauri-apps/api/core', () => ({
  // `resolve_data_dir` answers by default: it is infrastructure every view
  // needs at startup, not behaviour any single test is asserting. A test that
  // overrides `invoke` wholesale must answer it too.
  invoke: vi.fn(async (command: string) =>
    command === 'resolve_data_dir' ? '/mock/app-data' : undefined
  ),
  convertFileSrc: vi.fn((path: string) => `https://asset.localhost/${path}`),
}))

vi.mock('@tauri-apps/api/path', () => ({
  appDataDir: vi.fn().mockResolvedValue('/mock/app-data'),
  join: vi.fn((...parts: string[]) => Promise.resolve(parts.join('/'))),
}))

vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(),
  save: vi.fn(),
}))

vi.mock('@tauri-apps/plugin-fs', () => ({
  copyFile: vi.fn(),
  mkdir: vi.fn(),
  readFile: vi.fn(),
  writeFile: vi.fn(),
  remove: vi.fn(),
  stat: vi.fn().mockResolvedValue({ size: 0 }),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi
    .fn()
    .mockImplementation((_eventName: string, _callback: unknown) => Promise.resolve(vi.fn())),
}))

// The in-progress registry is app-wide state: a process one test started and
// never finished must not show as running in the next test.
// Imported late on purpose: a static import here would load the registry with
// this file's `listen` mock, before a test file installs its own.
afterEach(async () => {
  const { resetInflightForTests } = await import('$lib/inflight.svelte')
  resetInflightForTests()
})
