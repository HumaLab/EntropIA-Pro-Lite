export type DesktopPlatform = 'linux' | 'windows' | 'macos' | 'unknown'

type NavigatorWithUserAgentData = Navigator & {
  userAgentData?: {
    platform?: string
  }
}

function normalizePlatform(value: string | null | undefined): DesktopPlatform {
  const normalized = value?.trim().toLowerCase() ?? ''

  if (normalized.includes('linux')) return 'linux'
  if (normalized.includes('mac') || normalized.includes('darwin')) return 'macos'
  if (normalized.includes('win')) return 'windows'
  return 'unknown'
}

export function resolveDesktopPlatform(nav: NavigatorWithUserAgentData = navigator): DesktopPlatform {
  const userAgentDataPlatform = nav.userAgentData?.platform
  const platform = normalizePlatform(userAgentDataPlatform || nav.platform || nav.userAgent)
  return platform
}
