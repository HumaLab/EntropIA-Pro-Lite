import StarterKit from '@tiptap/starter-kit'
import Underline from '@tiptap/extension-underline'
import Link from '@tiptap/extension-link'
import Placeholder from '@tiptap/extension-placeholder'

/**
 * The note editor's schema. A factory, so the component and the tests that pin
 * the HTML a note is stored as mount exactly the same extensions.
 */
export function createNoteExtensions(placeholder = '') {
  return [
    StarterKit.configure({
      heading: { levels: [1, 2, 3] },
    }),
    Underline,
    Link.configure({
      openOnClick: false,
      autolink: false,
      HTMLAttributes: {
        rel: 'noopener noreferrer nofollow',
        target: '_blank',
      },
    }),
    Placeholder.configure({ placeholder }),
  ]
}
