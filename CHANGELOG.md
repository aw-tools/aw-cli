# Changelog

Every change a user of `aw` would notice is listed here, newest first. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `aw init` creates a workspace from the template at its latest stable release;
  `--template <url>@<ref>` picks another (#27)
- `aw bootstrap` clones the missing repositories, applies the git identity,
  links skills and activates the workspace hooks (#5)
- `aw doctor` checks the tools on your path, each remote's pinned branch and
  every skill link (#16)
- `aw status` reports each repository's presence and working tree, with `--json`
  and `--exit-code` (#8)
- `aw status` counts commits ahead of and behind upstream per repository (#9)
- `aw status` reports configuration drift (#12)
- `aw status` reports the health of every skill link (#13)
- `aw sync` fetches every repository and relinks skills; a failed fetch exits
  non-zero (#14)
- `aw sync` fetches the workspace layer too, and `aw status` ages each upstream
  row from the last fetch (#23)
- `aw sync` fetches several repositories at once; `--concurrency N` or
  `sync.concurrency` sets the number (#25)
- `aw adopt` adds an existing checkout to the manifest (#10)
- A repository's `skills` entry takes `dirs` and `only` to choose source
  directories and an allowlist (#18)
- `agents = true` on a repository links its agent definitions into
  `.claude/agents` (#24)
- `aw doctor` and `aw status` report a pre-commit hook that is configured but
  cannot run (#20)
- `aw doctor` and `aw adopt` name the file that declares a repository's delivery
  model (#22)

### Changed

- The crate is `aw-cli` in the `aw-tools` organisation; install with
  `cargo install --git https://github.com/aw-tools/aw-cli aw-cli` (#26)
- An `only` entry that matches no skill is reported, and a bad `--name` stops
  `aw init` before it creates anything (#19)

### Fixed

- A git command that times out takes its helper processes with it, so no socket
  or terminal stays held (#21)
