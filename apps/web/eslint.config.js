import js from '@eslint/js';
import tailwind from 'eslint-plugin-better-tailwindcss';
import svelte from 'eslint-plugin-svelte';
import globals from 'globals';
import ts from 'typescript-eslint';
export default [
  { ignores: ['.svelte-kit/**', 'build/**', 'test-results/**', 'playwright-report/**'] },
  js.configs.recommended,
  ...ts.configs.recommended,
  ...svelte.configs['flat/recommended'],
  { languageOptions: { globals: { ...globals.browser, ...globals.node } } },
  { files: ['**/*.svelte'], languageOptions: { parserOptions: { parser: ts.parser } } },
  {
    files: ['**/*.svelte'],
    plugins: { 'better-tailwindcss': tailwind },
    settings: { 'better-tailwindcss': { entryPoint: 'src/app.css', detectComponentClasses: true } },
    rules: { ...tailwind.configs.correctness.rules },
  },
];
