// Conventional Commits as this repo writes them: `type(scope)!: subject`,
// with a lower-case scope and a subject that starts lowercase unless it
// starts with an acronym (README, CI, OSC). Run it with scripts/lint-commits.
export default {
  extends: ['@commitlint/config-conventional'],
  // Merges keep their `Merge …` subjects; commitlint's own ignores only
  // cover git's wordings (`Merge branch …`, `Merge pull request …`).
  ignores: [(message) => message.startsWith('Merge ')],
  plugins: [
    {
      rules: {
        // subject-case's sentence-case check can't tell `Draw …` from
        // `README, …`, so this only rejects a capital followed by lowercase.
        'subject-lowercase-start': ({ subject }) => [
          !subject || !/^[A-Z][a-z]/.test(subject),
          'subject must start lowercase (acronyms like README or CI are fine)',
        ],
      },
    },
  ],
  rules: {
    'subject-case': [0],
    'subject-lowercase-start': [2, 'always'],
    // Not kebab-case: that rejects `e2e`.
    'scope-case': [2, 'always', 'lower-case'],
    'header-max-length': [2, 'always', 100],
    // Bodies are wrapped by hand; Dependabot's carry long URLs and YAML.
    'body-max-line-length': [0],
    'footer-max-line-length': [0],
  },
};
