# Contributing

`aw` is in active early development and is **not accepting external
contributions** at this time.

This may change once the project reaches a stable release.

**Security vulnerabilities** should not be reported through public channels. See
[SECURITY.md](SECURITY.md) for reporting instructions.

## Design invariants

The contract `aw` implements lives in the
[agentic-workspace](https://github.com/aw-tools/agentic-workspace) repository,
with a conformance suite beside it. Read the contract before changing what a
verb does: a behaviour change that breaks a suite fixture is a contract change,
and those are made there first.

## Commits

Commit subjects and pull request titles take the form `type(scope): subject`,
where the type is one of `feat`, `fix`, `refactor`, `chore`, `doc`, `deps`,
`test` or `ci`. The scope is optional and a trailing `!` marks a breaking
change. Branch names take one of six prefixes: `feat`, `fix`, `refactor`, `doc`,
`ci` or `deps`.

Pull requests are squash merged, so the title becomes the only commit subject
that reaches `main`, and the release tooling reads those subjects back out of
the log. The `PR health check` workflow fails a pull request whose title does
not match. Fixing the title in place turns the check green; no push is needed.

## Changelog

A pull request that changes what a user of the binary sees or can do adds one
entry under `[Unreleased]` in `CHANGELOG.md`, in the same pull request. The
entry is one sentence under twenty-five words, saying what the user can do or no
longer suffers, naming only what they type or see, and ending with the pull
request number: `(#12)`. A change nobody using the binary would notice adds
nothing; label the pull request `no-changelog` so the check passes.

## Prose

`README.md`, `CHANGELOG.md` and every document under `docs/` are linted by Vale
against the rules in `.vale/styles`, and each document has a word budget in
`docs/budget.txt`. Run `just vale` and `just budget` before pushing. Raise a
budget line in a commit that says why. When a rule changes, `just test-lint`
proves it still fires on its fixture.
