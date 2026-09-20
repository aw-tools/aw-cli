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

## Development

Run `just setup` once per clone: it points git at `.githooks`, whose pre-commit
hook checks formatting with dprint. Run `just ci` before pushing: it is the same
pipeline the `CI` workflow runs, so a green run here is the check passing there.
The Rust toolchain is pinned in `rust-toolchain.toml`, and throwaway files go
under `tmp/`, which is ignored.

A change to what a verb does is not finished when the suite passes. Build the
binary, install it, and drive the changed behaviour through a real workspace.
Every test fixture is a workspace created moments earlier. Anything that depends
on accumulated state, such as fetch ages, history or drift, reads correct in the
suite and wrong in the field.

## Commits

Commit subjects and pull request titles take the form `type(scope): subject`,
where the type is one of `feat`, `fix`, `refactor`, `chore`, `doc`, `deps`,
`test` or `ci`. The scope is optional and a trailing `!` marks a breaking
change. Branch names take one of six prefixes: `feat`, `fix`, `refactor`, `doc`,
`ci` or `deps`.

Pull requests are squash merged, so the title becomes the only commit subject
that reaches `main`, and the release tooling reads those subjects back out of
the log. The `Pull request health` workflow reports a `PR health check` status,
and that check fails when the title does not match. Fixing the title in place
turns it green; no push is needed.

Commits are signed.

## Delivery

Every change reaches `main` through a pull request: a signed commit on a
prefixed branch, pushed, then opened as a draft.

The body says what the change does, then why, in short paragraphs of plain
language. Write about the change, not about the person making it. No em dashes,
and no test plan, validation or monitoring section: the checks report
themselves.

## Changelog

A pull request that changes what a user of the binary sees or can do adds one
entry under `[Unreleased]` in `CHANGELOG.md`, in the same pull request. The
entry is one sentence under twenty-five words, saying what the user can do or no
longer suffers, naming only what they type or see, and ending with the pull
request number: `(#12)`. A change nobody using the binary would notice adds
nothing; label the pull request `no-changelog` so the check passes. The check
only runs when `src/` changes, so a pull request that touches no code never
meets it and needs no label.

## Prose

`README.md`, `CHANGELOG.md` and every document under `docs/` are linted by Vale
against the rules in `.vale/styles`, and each document has a word budget in
`docs/budget.txt`. Run `just vale` and `just budget` before pushing. Raise a
budget line in a commit that says why. When a rule changes, `just test-lint`
proves it still fires on its fixture.
