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
    case 'biblioteca':
      return t('nav.biblioteca')
    case 'bibliography-work':
      return view.title
    case 'settings':
      return t('nav.settings')
    case 'navegador':
      return t('nav.navegador')
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
    // A work is a book in the library's books, whatever page it opens on.
    case 'biblioteca':
    case 'bibliography-work':
      return 'books'
    case 'settings':
      return 'settings'
    case 'navegador':
      return 'browser'
    default: {
      const exhaustive: never = view
      return exhaustive
    }
  }
}
