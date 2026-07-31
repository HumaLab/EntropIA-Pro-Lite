import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LOCAL_ML } from './capabilities'
import {
  getRuntimeStatus,
  getRuntimeBootstrapPlan,
  repairRuntime,
  onRuntimeStatus,
  onRuntimeProgress,
  runtimeCanBootstrapAutomatically,
  runtimeBlocksCurrentUse,
  runtimeNeedsAttention,
  shouldShowRuntimeRepairAction,
  type RuntimeStatus,
} from './runtime'

const { invoke } = await import('@tauri-apps/api/core')
const { listen } = await import('@tauri-apps/api/event')

describe('runtime client', () => {
  beforeEach(() => {
    vi.resetAllMocks()
  })

  it.runIf(LOCAL_ML)('getRuntimeStatus calls the runtime status command', async () => {
    const status: RuntimeStatus = {
      state: 'healthy',
      packVersion: '2026.05.0',
      repairNeeded: false,
      repairAvailable: true,
      summary: 'Runtime listo',
      blockedCapabilities: [],
      details: [],
      guidance: [],
      bootstrapEligible: false,
      bootstrapRequired: false,
      activeOperation: null,
    }
    vi.mocked(invoke).mockResolvedValueOnce(status)

    const result = await getRuntimeStatus()

    expect(invoke).toHaveBeenCalledWith('runtime_get_status')
    expect(result).toEqual(status)
  })

  it.runIf(LOCAL_ML)('repairRuntime calls the runtime repair command', async () => {
    const status: RuntimeStatus = {
      state: 'repairing',
      packVersion: '2026.05.0',
      repairNeeded: true,
      repairAvailable: true,
      summary: 'Reparando runtime',
      blockedCapabilities: ['ocr'],
      details: [],
      guidance: [],
      bootstrapEligible: false,
      bootstrapRequired: false,
      activeOperation: null,
    }
    vi.mocked(invoke).mockResolvedValueOnce(status)

    await repairRuntime()

    expect(invoke).toHaveBeenCalledWith('runtime_repair')
  })

  it('getRuntimeBootstrapPlan calls the bootstrap planning command', async () => {
    vi.mocked(invoke).mockResolvedValueOnce({
      eligible: true,
      required: true,
      source: 'trusted_remote',
      packVersion: '2026.05.1',
      summary: 'Descarga remota planificada',
      reason: null,
      remoteSource: {
        manifestUrl: 'https://example.com/bootstrap.json',
        publicKeyId: 'entropia-root',
      },
      download: {
        archiveUrl: 'https://example.com/runtime-pack.archive',
        archiveSha256: 'remote-sha',
        archiveSize: 1024,
        archivePath: '/tmp/runtime/.downloads/2026.05.1/runtime-pack.archive',
        stagingPath: '/tmp/runtime/.2026.05.1.staging',
        resumeMetadataPath: '/tmp/runtime/.downloads/2026.05.1/resume.json',
      },
    })

    const result = await getRuntimeBootstrapPlan()

    expect(invoke).toHaveBeenCalledWith('runtime_get_bootstrap_plan')
    expect(result.source).toBe('trusted_remote')
  })

  it.runIf(LOCAL_ML)('onRuntimeStatus subscribes to runtime status events', async () => {
    const callback = vi.fn()

    await onRuntimeStatus(callback)

    expect(listen).toHaveBeenCalledWith('runtime://status', expect.any(Function))
  })

  it.runIf(LOCAL_ML)('onRuntimeProgress subscribes to runtime progress events', async () => {
    const callback = vi.fn()

    await onRuntimeProgress(callback)

    expect(listen).toHaveBeenCalledWith('runtime://progress', expect.any(Function))
  })

  it.runIf(LOCAL_ML)('runtimeNeedsAttention is true for damaged, fixture, and incompatible states', () => {
    expect(runtimeNeedsAttention({ state: 'damaged' } as RuntimeStatus)).toBe(true)
    expect(runtimeNeedsAttention({ state: 'fixture' } as RuntimeStatus)).toBe(true)
    expect(runtimeNeedsAttention({ state: 'incompatible' } as RuntimeStatus)).toBe(true)
    expect(runtimeNeedsAttention({ state: 'blocked_offline' } as RuntimeStatus)).toBe(true)
    expect(runtimeNeedsAttention({ state: 'healthy' } as RuntimeStatus)).toBe(false)
  })

  it.runIf(LOCAL_ML)('runtimeBlocksCurrentUse treats fixture as packaging-only only when local deps are ready', () => {
    expect(runtimeBlocksCurrentUse({ state: 'fixture' } as RuntimeStatus, true)).toBe(false)
    expect(runtimeBlocksCurrentUse({ state: 'fixture' } as RuntimeStatus, false)).toBe(true)
    expect(runtimeBlocksCurrentUse({ state: 'blocked_source_unavailable' } as RuntimeStatus, true)).toBe(true)
    expect(runtimeBlocksCurrentUse({ state: 'blocked_source_unavailable' } as RuntimeStatus, true, true)).toBe(false)
    expect(runtimeBlocksCurrentUse({ state: 'blocked_offline' } as RuntimeStatus, true, true)).toBe(false)
    expect(runtimeBlocksCurrentUse({ state: 'damaged' } as RuntimeStatus, true)).toBe(true)
    expect(runtimeBlocksCurrentUse({ state: 'healthy' } as RuntimeStatus, true)).toBe(false)
  })

  it.runIf(LOCAL_ML)('shouldShowRuntimeRepairAction hides repair for fixture and incompatible runtime states', () => {
    expect(
      shouldShowRuntimeRepairAction({
        state: 'damaged',
        repairAvailable: true,
      } as RuntimeStatus)
    ).toBe(true)

    expect(
      shouldShowRuntimeRepairAction({
        state: 'fixture',
        repairAvailable: true,
      } as RuntimeStatus)
    ).toBe(false)

    expect(
      shouldShowRuntimeRepairAction({
        state: 'incompatible',
        repairAvailable: true,
      } as RuntimeStatus)
    ).toBe(false)
  })

  it.runIf(LOCAL_ML)('runtimeCanBootstrapAutomatically requires an eligible blocked runtime', () => {
    expect(
      runtimeCanBootstrapAutomatically({
        state: 'damaged',
        bootstrapEligible: true,
      } as RuntimeStatus),
    ).toBe(true)

    expect(
      runtimeCanBootstrapAutomatically({
        state: 'blocked_source_unavailable',
        bootstrapEligible: false,
      } as RuntimeStatus),
    ).toBe(false)
  })

  it('shouldShowRuntimeRepairAction hides repair for blocked bootstrap states', () => {
    expect(
      shouldShowRuntimeRepairAction({
        state: 'blocked_source_unavailable',
        repairAvailable: true,
      } as RuntimeStatus),
    ).toBe(false)

    expect(
      shouldShowRuntimeRepairAction({
        state: 'blocked_offline',
        repairAvailable: true,
      } as RuntimeStatus),
    ).toBe(false)
  })

  it.runIf(!LOCAL_ML)('keeps managed runtime operations inert in Lite', async () => {
    const status = await getRuntimeStatus()
    const repaired = await repairRuntime()
    await onRuntimeStatus(vi.fn())
    await onRuntimeProgress(vi.fn())

    expect(status.state).toBe('healthy')
    expect(repaired.state).toBe('healthy')
    expect(invoke).not.toHaveBeenCalledWith('runtime_get_status')
    expect(invoke).not.toHaveBeenCalledWith('runtime_repair')
    expect(listen).not.toHaveBeenCalled()
    expect(runtimeNeedsAttention({ state: 'damaged' } as RuntimeStatus)).toBe(false)
    expect(runtimeBlocksCurrentUse({ state: 'damaged' } as RuntimeStatus, false)).toBe(false)
    expect(
      shouldShowRuntimeRepairAction({ state: 'damaged', repairAvailable: true } as RuntimeStatus),
    ).toBe(false)
    expect(
      runtimeCanBootstrapAutomatically({ state: 'damaged', bootstrapEligible: true } as RuntimeStatus),
    ).toBe(false)
  })
})
