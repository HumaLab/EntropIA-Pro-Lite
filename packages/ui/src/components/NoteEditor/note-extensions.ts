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
    // Tiptap 3's StarterKit also carries link, underline, a list keymap and a
    // trailing node. Link and underline are configured below, once; the other
    // two would change how a note is edited and add an empty paragraph after a
    // note that ends in a list, a quote or a code block.
    StarterKit.configure({
      heading: { levels: [1, 2, 3] },
      link: false,
      underline: false,
      listKeymap: false,
      trailingNode: false,
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
