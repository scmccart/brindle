// Conventional Commits as this repo writes them: `type(scope)!: subject`,
// with a lower-case scope and a subject that starts lowercase unless it
// starts with an acronym (README, CI, OSC). Run it with scripts/lint-commits.
export default {
  extends: ['@commitlint/config-conventional'],
  // Merges keep their `Merge …` subjects; commitlint's own ignores only
  // cover git's wordings (`Merge branch …`, `Merge pull request …`).
  // The two older commits whose wrapped bodies read as trailers, from before
  // footer-leading-blank was an error, so a whole-history run still passes.
  ignores: [
    (message) => message.startsWith('Merge '),
    (message) =>
      [
        "fix: name Windows tabs after their program, not the console's path title\n",
        'build: version information in brindle.exe\n',
      ].some((header) => message.startsWith(header)),
  ],
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
    // Errors, not config-conventional's warnings, so the commit-msg hook
    // stops a body line that reads as a trailer (`id-token: write` after a
    // wrap) before the commit is pushed and can only be fixed by a rewrite.
    'body-leading-blank': [2, 'always'],
    'footer-leading-blank': [2, 'always'],
    // Bodies are wrapped by hand; Dependabot's carry long URLs and YAML.
    'body-max-line-length': [0],
    'footer-max-line-length': [0],
  },
};
