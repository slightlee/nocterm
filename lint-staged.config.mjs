export default {
  '*.{js,mjs,cjs,ts,tsx}': ['eslint --fix --max-warnings 0', 'prettier --write'],
  '*.css': ['stylelint --fix', 'prettier --write'],
  '*.{json,jsonc,md,yaml,yml,html}': 'prettier --write',
  '*.rs': () => 'cargo fmt --all --check',
};
