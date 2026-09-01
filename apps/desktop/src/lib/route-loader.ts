import type { Component } from 'svelte'
import type { View } from './navigation'

export type LazyViewName = Exclude<View['name'], 'collections'>

type RouteModule = {
  default: Component<any>
}

export function createRouteLoader<Name extends string, Module>(
  loaders: Record<Name, () => Promise<Module>>,
): (name: Name) => Promise<Module> {
  const cache = new Map<Name, Promise<Module>>()

  return (name) => {
    const cached = cache.get(name)
    if (cached) return cached
    // Note: retry re-runs the import and is effective for promise-level
    // failures. A browser-level network failure is cached per URL in
    // Chromium's module map, so recovering from it requires a page reload.
    const pending = loaders[name]().catch((error) => {
      if (cache.get(name) === pending) cache.delete(name)
      throw error
    })
    cache.set(name, pending)
    return pending
  }
}

const loadCachedRoute = createRouteLoader<LazyViewName, RouteModule>({
  collection: () => import('../views/CollectionView.svelte'),
  item: () => import('../views/ItemView.svelte'),
  'db-browser': () => import('../views/DbBrowserView.svelte'),
  'rag-chat': () => import('../views/RagChatView.svelte'),
  settings: () => import('../views/SettingsView.svelte'),
})

export function loadRouteView(name: LazyViewName): Promise<RouteModule> {
  return loadCachedRoute(name)
}
