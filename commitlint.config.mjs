export const isEnglishAsciiText = (value) =>
  typeof value === 'string' &&
  value.length > 0 &&
  [...value].every((character) => {
    const codePoint = character.codePointAt(0);
    return codePoint >= 0x20 && codePoint <= 0x7e;
  });

export default {
  extends: ['@commitlint/config-conventional'],
  plugins: [
    {
      rules: {
        'subject-english-ascii': ({ subject }) => [
          isEnglishAsciiText(subject),
          'subject must use English ASCII text',
        ],
      },
    },
  ],
  rules: {
    'header-max-length': [2, 'always', 100],
    'scope-empty': [2, 'always'],
    'subject-empty': [2, 'never'],
    'subject-english-ascii': [2, 'always'],
    'subject-full-stop': [2, 'never', '.'],
    'type-enum': [
      2,
      'always',
      [
        'build',
        'chore',
        'ci',
        'docs',
        'feat',
        'fix',
        'perf',
        'refactor',
        'revert',
        'style',
        'test',
      ],
    ],
  },
};
