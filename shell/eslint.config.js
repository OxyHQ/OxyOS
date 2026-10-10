/**
 * MINIMAL ESLint config. Biome (`biome.json`) formats and lints the shell; this
 * file holds only the React Compiler rules of `eslint-plugin-react-hooks` 7
 * (`purity`, `refs`, `immutability`, `set-state-in-effect`, `static-components`,
 * ...), which Biome has no equivalent for. They ran at their recommended levels
 * before the move to Biome. `rules-of-hooks` and `exhaustive-deps` are NOT
 * here: Biome's `useHookAtTopLevel` and `useExhaustiveDependencies` run them.
 * Do not add rules Biome can enforce.
 */
import tsParser from '@typescript-eslint/parser';
import reactHooks from 'eslint-plugin-react-hooks';
import { defineConfig, globalIgnores } from 'eslint/config';

const reactCompilerRules = Object.fromEntries(
  Object.entries(reactHooks.configs.flat.recommended.rules).filter(
    ([name]) => name !== 'react-hooks/rules-of-hooks' && name !== 'react-hooks/exhaustive-deps',
  ),
);

export default defineConfig([
  globalIgnores(['build', 'web-build', 'dist', 'src-tauri']),
  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      parser: tsParser,
      ecmaVersion: 'latest',
      sourceType: 'module',
      parserOptions: { ecmaFeatures: { jsx: true } },
    },
    linterOptions: { reportUnusedDisableDirectives: 'error' },
    plugins: { 'react-hooks': reactHooks },
    rules: reactCompilerRules,
  },
]);
