# Changelog

Every change a user of `aw` would notice is listed here, newest first. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Install from Homebrew with `brew install aw-tools/tap/aw-cli`; each release
  points the tap at itself (#40)

## [0.1.0] - 2026-09-19

### Added

- `aw init` creates a workspace from the template at its latest stable release;
  `--template <url>@<ref>` picks another (#27)
- `aw init` rejects a bad `--name` before it creates anything, and reports an
  `only` entry that matches no skill (#19)
- `aw bootstrap` clones the missing repositories, applies the git identity,
  links skills and activates the workspace hooks (#5)
- `aw doctor` checks the tools on your path, each remote's pinned branch and
  every skill link (#16)
- `aw doctor` and `aw status` report a pre-commit hook that is configured but
  cannot run (#20)
- `aw doctor` and `aw adopt` name the file that declares a repository's delivery
  model (#22)
- `aw status` reports each repository's presence and working tree, with `--json`
  and `--exit-code` (#8)
- `aw status` counts commits ahead of and behind upstream per repository (#9)
- `aw status` reports configuration drift (#12)
- `aw status` reports the health of every skill link (#13)
- `aw sync` fetches every repository and relinks skills; a failed fetch exits
  non-zero (#14)
- `aw sync` also fetches the workspace layer, and `aw status` shows how long ago
  each upstream row was fetched (#23)
- `aw sync` fetches several repositories at once; `--concurrency N` or
  `sync.concurrency` sets the number (#25)
- `aw adopt` adds an existing checkout to the manifest (#10)
- A repository's `skills` entry takes `dirs` and `only` to choose source
  directories and an allowlist (#18)
- `agents = true` on a repository links its agent definitions into
  `.claude/agents` (#24)

## [0.1.0-rc.2] - 2026-09-19

### Changed

- Release archives drop the version from their names, so the README install
  command works for every release (#37)

## [0.1.0-rc.1] - 2026-09-19

### Added

- `aw init` creates a workspace from the template at its latest stable release;
  `--template <url>@<ref>` picks another (#27)
- `aw init` rejects a bad `--name` before it creates anything, and reports an
  `only` entry that matches no skill (#19)
- `aw bootstrap` clones the missing repositories, applies the git identity,
  links skills and activates the workspace hooks (#5)
- `aw doctor` checks the tools on your path, each remote's pinned branch and
  every skill link (#16)
- `aw doctor` and `aw status` report a pre-commit hook that is configured but
  cannot run (#20)
- `aw doctor` and `aw adopt` name the file that declares a repository's delivery
  model (#22)
- `aw status` reports each repository's presence and working tree, with `--json`
  and `--exit-code` (#8)
- `aw status` counts commits ahead of and behind upstream per repository (#9)
- `aw status` reports configuration drift (#12)
- `aw status` reports the health of every skill link (#13)
- `aw sync` fetches every repository and relinks skills; a failed fetch exits
  non-zero (#14)
- `aw sync` also fetches the workspace layer, and `aw status` shows how long ago
  each upstream row was fetched (#23)
- `aw sync` fetches several repositories at once; `--concurrency N` or
  `sync.concurrency` sets the number (#25)
- `aw adopt` adds an existing checkout to the manifest (#10)
- A repository's `skills` entry takes `dirs` and `only` to choose source
  directories and an allowlist (#18)
- `agents = true` on a repository links its agent definitions into
  `.claude/agents` (#24)
