import '@testing-library/jest-dom'

// happy-dom 20 no longer exposes window.prompt, but tests assert that
// production code never falls back to it with `vi.spyOn(window, 'prompt')`.
// Provide the missing browser API as a no-op so the spy targets a real
// function and its "never called" assertion keeps meaning.
if (typeof window.prompt !== 'function') {
  window.prompt = () => null
}
