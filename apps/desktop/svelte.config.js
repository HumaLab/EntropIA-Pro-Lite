// svelte-check 4.7 reads Svelte options from svelte.config.js, or else from the
// vite-plugin-svelte instance in vite.config, which v5 does not expose. Keep an
// explicit (empty) config so type checking never depends on that lookup.
export default {}
