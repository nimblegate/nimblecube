# CLAUDE.md

Conventions for AI assistants working in this repo.

## Writing style

- No em dashes (the long dash) anywhere in text. Restructure the sentence with a colon, a comma, parentheses, or a period instead.
- En dash (`–`) only where it is genuinely the right character: numeric or range spans like `10–20` or `A–B`. Do not use it as a casual connector between clauses or after every other word.
- Plain hyphens stay where they belong: compound words (`no_std`, `integer-only`, `bare-metal`) and code.

## Commit messages

- Keep them terse. State only what changed or was fixed. No explanation, no rationale, no "because".
- Good: `Fix nearest tie-break`, `Correct firmware footprint to 2.9 KB`, `Add PSRAM capacity bench`.
- Bad: `Fix nearest tie-break so that the matched query path returns the lowest id when distances are equal, which was needed because ...`
