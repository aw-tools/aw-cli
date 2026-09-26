# Agent instructions

Read `CONTRIBUTING.md` before changing anything. Its rules on development,
delivery, commits, the changelog and prose bind every contributor, human or
agent.

Four things are yours in particular:

- Show the diff and the local `just ci` run, and wait for a person's go, before
  every commit and every push. Larger work may instead land as a series of local
  commits, announced when the work starts and handed over with the base and head
  SHAs and the `git log --stat` and `git diff` commands for that range. The go
  before the push still applies.
- Stop once the draft pull request is open. Marking it ready for review and
  merging it are never yours to do, and never unattended.
- Add nothing to a commit beyond its message: no co-author trailer, no tooling
  footer, no attribution to whatever wrote the change. The same goes for pull
  request bodies and code comments.
- Keep scratch files under `tmp/`.
