/**
 * The current language, as state a template can depend on.
 *
 * The `locale` store in i18n.ts stays the one to set and subscribe to. This is
 * its mirror as a rune, read by `t`: a store read through `get()` is invisible
 * to Svelte, so a template calling `t` never re-rendered when the language
 * changed, and nothing remounts the views to make up for it. Reading this
 * instead makes every `t` in a template, `$derived` or effect follow the
 * language by itself.
 */
export const localeState = $state<{ current: string }>({ current: 'es' })
