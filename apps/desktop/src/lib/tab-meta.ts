/** A tab's short title and icon: "the document, collection, investigation,
 *  or section name of its current view" (spec, Tabs). */
import type { View } from './navigation'
import { t } from './i18n'
import type { ActionIconName } from '@entropia/ui'

export function tabTitle(view: View): string {
  switch (view.name) {
    case 'home':
      return t('home.title')
    case 'collections':
      return t('nav.collections')
    case 'collection':
      return view.collectionName
    case 'item':
      return view.itemTitle
    case 'db-browser':
      return t('nav.dbBrowser')
    case 'rag-chat':
      return t('nav.ragChat')
    case 'research':
      return t('nav.research')
    case 'investigation':
      return view.title
    case 'writing':
      return view.documentTitle ?? t('writing.title')
    case 'settings':
      return t('nav.settings')
    default: {
      const exhaustive: never = view
      return exhaustive
    }
  }
}

export function tabIcon(view: View): ActionIconName {
  switch (view.name) {
    case 'home':
      return 'home'
    case 'collections':
    case 'collection':
      return 'folder'
    case 'item':
      return 'file'
    case 'db-browser':
      return 'database'
    case 'rag-chat':
      return 'message-circle'
    case 'research':
    case 'investigation':
      return 'research'
    case 'writing':
      return 'edit'
    case 'settings':
      return 'settings'
    default: {
      const exhaustive: never = view
      return exhaustive
    }
  }
}
