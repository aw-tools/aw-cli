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
