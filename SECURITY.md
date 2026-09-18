# Security

## Threat model

`aw` is a **local single-user CLI tool**. It runs under the invoking user's
permissions, has no network listener, no daemon and no authentication. Its only
network activity is the `git` it spawns: cloning a template on `aw init`,
cloning and fetching managed repositories on `aw bootstrap` and `aw sync`, and
probing remotes on `aw doctor`.

The primary threats are:

- **A malicious template.** `aw init` clones a template from a URL the user
  gives, or the built-in default, and without an explicit ref seeds from the
  highest stable tag found in that clone, never from anything the remote
  advertises separately. The template's contents become the workspace, and
  `aw bootstrap` sets `core.hooksPath` to the template's hook directory when one
  exists, so a hostile template can run code on the user's next commit.
- **Credential leakage** through a template or manifest URL.
- **A hung or runaway subprocess** from an unreachable remote.

## Trust boundaries

| Input surface                 | Trust level                      | Validation                                                                                 |
| ----------------------------- | -------------------------------- | ------------------------------------------------------------------------------------------ |
| CLI arguments                 | Trusted (user-invoked)           | Clap argument parsing                                                                      |
| Template URL and ref          | Partially trusted                | HTTP(S) URLs with embedded credentials, query strings or fragments are rejected            |
| Template contents (`aw init`) | Untrusted until the user reads   | Not executed by `aw`; hooks only take effect once `aw bootstrap` runs in the new workspace |
| Manifest (`workspace.toml`)   | Trusted (user-authored, tracked) | TOML parsing via `serde`; repository URLs are passed to `git` and `garden` as arguments    |
| Git and garden output         | Trusted (local tools)            | Parsed for reporting only                                                                  |

## Security measures

- **No shell.** Every subprocess is a `std::process::Command` with an explicit
  argument list. `aw` never composes a shell string.
- **Bounded subprocesses.** Template clones and fetches run under a timeout, and
  the child leads its own process group so a timeout tears down the whole tree
  rather than orphaning it.
- **No embedded credentials.** A template URL carrying a user, password, query
  string or fragment is refused with a message pointing at a git credential
  helper.
- **Working trees are never written.** `aw` clones, fetches, sets git
  configuration and creates symlinks for skills. It never checks out, merges,
  rebases or edits a file inside a managed repository.
- **Code safety.** `unsafe_code = "deny"` globally; clippy pedantic at warn;
  dependencies audited by `cargo-deny` in CI (advisories, licences, bans); every
  third-party GitHub action pinned to a commit hash.

## Assumptions

- The user reads a template before running `aw bootstrap` inside it, the same
  way they would read a repository before running its build. A first-run consent
  prompt and a content check against the tag are planned and not yet built.
- `git` and `garden` on `PATH` are the user's own installs and are trusted.
- Remotes are reached over the transports git is configured for; `aw` adds no
  transport of its own.

## Reporting vulnerabilities

If you discover a security vulnerability, please report it through
[GitHub Security Advisories](../../security/advisories/new) or contact the
maintainer directly. Do not report security vulnerabilities through public
channels.

`aw` is not currently accepting external contributions (see
[CONTRIBUTING.md](CONTRIBUTING.md)), but security reports are always welcome.
