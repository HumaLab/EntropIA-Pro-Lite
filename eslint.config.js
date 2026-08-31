import eslint from '@eslint/js'
import globals from 'globals'
import tseslint from 'typescript-eslint'
import svelte from 'eslint-plugin-svelte'

export default tseslint.config(
  eslint.configs.recommended,
  ...tseslint.configs.recommended,
  ...svelte.configs.recommended,
  // Formatting is owned by prettier-plugin-svelte; disable conflicting stylistic rules.
  ...svelte.configs.prettier,
  {
    rules: {
      '@typescript-eslint/no-unused-vars': [
        'warn',
        // ignoreRestSiblings: the codebase strips fields via rest destructuring ({ a, ...rest }) => rest.
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_', ignoreRestSiblings: true },
      ],
      '@typescript-eslint/no-explicit-any': 'warn',
    },
  },
  {
    files: ['**/*.svelte'],
    languageOptions: {
      // Components run in the Tauri webview; no-undef needs the browser globals.
      globals: { ...globals.browser },
      parserOptions: {
        // Parse <script lang="ts"> blocks with the typescript-eslint parser.
        parser: tseslint.parser,
        extraFileExtensions: ['.svelte'],
      },
    },
    rules: {
      // Bare expressions inside $effect/$derived.by are the runes dependency-tracking idiom here.
      '@typescript-eslint/no-unused-expressions': 'off',
      // Reactive Maps follow a copy-on-write reassignment convention; flagged news are local caches.
      'svelte/prefer-svelte-reactivity': 'off',
      // Empty catch is the established best-effort pattern around localStorage access.
      'no-empty': ['error', { allowEmptyCatch: true }],
    },
  },
  // ─── Icon contract (see packages/ui/src/components/Button/ActionIcon.types.ts) ───
  // Every icon in the app goes through ActionIcon: one family, one geometry,
  // one closed size scale. These two rules are what keep that true.
  //
  // Both are `error`: the migration is done, so from here on a hand-written
  // icon is a build failure, not a note. Genuinely non-icon SVG (charts, viewer
  // overlays, third-party brand marks) belongs in the allowlist below.
  {
    files: ['apps/**/*.{ts,svelte}', 'packages/**/*.{ts,svelte}'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          paths: [
            {
              name: '@lucide/svelte',
              message:
                'Import icons through ActionIcon instead. Add the name to ACTION_ICON_NAMES and map it in ActionIcon.svelte.',
            },
          ],
        },
      ],
    },
  },
  {
    files: ['apps/**/*.svelte', 'packages/**/*.svelte'],
    rules: {
      'svelte/no-restricted-html-elements': [
        'error',
        {
          elements: ['svg'],
          message:
            'Hand-written <svg> drifts from the icon contract (stroke 2, 24 grid). Use <ActionIcon name="..." />. Data visualisations and viewer overlays are exempt — add the file to the allowlist in eslint.config.js.',
        },
      ],
    },
  },
  {
    // The allowlist. Every entry is SVG that is deliberately NOT an icon:
    //  - ActionIcon is the gate itself, and its in-house icons under ./icons are
    //    authored SVG by definition.
    //  - DocumentViewer draws annotation and layout geometry;
    //    CollectionAnalysisPanel draws a word cloud and a bar chart. Data, not icons.
    //  - AppShell carries the GitHub wordmark: a third-party brand mark, filled
    //    rather than stroked, on its own 16 grid. Redrawing it on our metric
    //    would misrepresent someone else's mark, so it stays verbatim.
    //  - Test hosts, mocks and fixtures stand in for real components and never ship.
    files: [
      'packages/ui/src/components/Button/ActionIcon.svelte',
      'packages/ui/src/components/Button/icons/*.svelte',
      'packages/ui/src/components/DocumentViewer/DocumentViewer.svelte',
      'apps/desktop/src/views/CollectionAnalysisPanel.svelte',
      'apps/desktop/src/layout/AppShell.svelte',
      '**/__tests__/**',
      '**/__mocks__/**',
      '**/__fixtures__/**',
    ],
    rules: {
      'no-restricted-imports': 'off',
      'svelte/no-restricted-html-elements': 'off',
    },
  },
  {
    ignores: [
      '**/node_modules/**',
      '**/dist/**',
      '**/build/**',
      '**/target/**',
      '**/.turbo/**',
      '**/.svelte-kit/**',
      '**/coverage/**',
    ],
  }
)
