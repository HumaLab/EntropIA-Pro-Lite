import { LOCAL_ML } from '$lib/capabilities'
import { requestSettingsTab } from '$lib/settings-tab-request'
import { workspace } from '$lib/workspace'

/** Opens Settings on Dependencias de IA, where the dictation engine is fixed. */
export function openDependenciesSettings(): void {
  requestSettingsTab('dependencias')
  workspace.navigateActive({ name: 'settings' })
}

/**
 * What a failed dictation offers next to its error. Only Pro has the
 * Dependencias de IA tab, so Lite shows the error alone.
 */
export const dictationHelp: (() => void) | undefined = LOCAL_ML
  ? openDependenciesSettings
  : undefined
