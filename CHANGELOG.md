# Changelog

Every change a user of `aw` would notice is listed here, newest first. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- `aw adopt` records the branch name itself when a tag or another branch shares
  its short name, instead of `heads/main` (#61)
- `aw init` refuses a template whose `.gitignore` does not deny everything by
  default, as the template contract requires (#62)
- `aw init` lists a template's `garden.yaml` before asking to use it, because
  `aw bootstrap` hands it to garden, which can run commands from it (#65)
- `aw bootstrap` passes every `workspace.toml` value to garden as written, so
  garden runs none of them as a shell command (#66)

## [0.4.0] - 2026-10-03

### Added

- `aw init` records `contract = 1` in `workspace.toml`, and a `[template]` block
  may hold `contract` alone (#51)
- `aw lint --all` checks every tracked artefact against the contract, printing
  each failure with the name of the rule it breaks (#52)
- `aw lint` checks the pending commit, catching a status moving backwards, an
  edited decision, or a finished note deleted without its outcome (#53)
- `aw lint` checks where each file sits and what it is named, and warns about
  open reports and untracked notes (#55)
- `aw lint` checks the house style when the registry has a `[state]` table, and
  `aw lint --remits` lists engagements by remit (#56)

## [0.4.0-rc.1] - 2026-10-03

### Added

- `aw init` records `contract = 1` in `workspace.toml`, and a `[template]` block
  may hold `contract` alone (#51)
- `aw lint --all` checks every tracked artefact against the contract, printing
  each failure with the name of the rule it breaks (#52)
- `aw lint` checks the pending commit, catching a status moving backwards, an
  edited decision, or a finished note deleted without its outcome (#53)
- `aw lint` checks where each file sits and what it is named, and warns about
  open reports and untracked notes (#55)
- `aw lint` checks the house style when the registry has a `[state]` table, and
  `aw lint --remits` lists engagements by remit (#56)

## [0.3.0] - 2026-09-29

### Added

- `aw init` shows a non-default template's hooks, executables, agent files and
  links, and asks before using it; `--trust-template` skips the question (#49)

### Fixed

- `aw init` clones the default template over HTTPS, so it works without a GitHub
  SSH key (#47)
- A failed `aw init` no longer leaves empty directories behind, and its error
  says what to check next (#48)

## [0.2.0] - 2026-09-25

### Added

- Each release updates the Homebrew tap, so `brew upgrade` finds it (#40)
- `aw fast-forward` fetches, then brings each clean repository on its default
  branch up to its upstream; `--dry-run` shows what would move (#44)

### Fixed

- `aw init` ends by pointing at `aw bootstrap`, which works for any template,
  instead of a script only some templates ship (#42)

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
