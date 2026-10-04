# Security

## Threat model

`aw` is a **local single-user CLI tool**. It runs under the invoking user's
permissions, has no network listener, no daemon and no authentication. Its only
network activity is the `git` it spawns: cloning a template on `aw init`,
cloning and fetching managed repositories on `aw bootstrap`, `aw sync` and
`aw fast-forward`, asking origin for its default branch on `aw fast-forward`,
and probing remotes on `aw doctor`.

The primary threats are:

- **A malicious template.** `aw init` clones a template from a URL the user
  gives, or the built-in default, and without an explicit ref seeds from the
  highest stable tag found in that clone, never from anything the remote
  advertises separately. The template's contents become the workspace.
  `aw bootstrap` runs `garden` on the template's `garden.yaml`, which can run
  shell commands, and sets `core.hooksPath` to the template's hook directory
  when one exists. A hostile template can therefore run code during
  `aw bootstrap` and on every commit after it. Before copying a template other
  than the default, `aw init` lists the hooks, executables, agent files and
  links it holds, folding a directory of more than five into a count, and asks
  whether to go on. The listing omits `garden.yaml`. `aw init` does not ask when
  the workspace already records that template. `--trust-template` skips both the
  listing and the question; without it and with no terminal to ask on, `aw init`
  prints the listing and exits with status 2.
- **Credential leakage** through a template or manifest URL.
- **A hung or runaway subprocess** from an unreachable remote.

## Trust boundaries

| Input surface                 | Trust level                      | Validation                                                                                                        |
| ----------------------------- | -------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| CLI arguments                 | Trusted (user-invoked)           | Clap argument parsing                                                                                             |
| Template URL and ref          | Partially trusted                | HTTP(S) URLs with embedded credentials, query strings or fragments are rejected                                   |
| Template contents (`aw init`) | Untrusted until the user reads   | Not executed by `aw init`; a template other than the default is listed and confirmed first (see the threat model) |
| Manifest (`workspace.toml`)   | Trusted (user-authored, tracked) | TOML parsing via `serde`; repository URLs are passed to `git` and `garden` as arguments                           |
| Git and garden output         | Trusted (local tools)            | Parsed for reporting only                                                                                         |

## Security measures

- **No shell.** Every subprocess is a `std::process::Command` with an explicit
  argument list. `aw` never composes a shell string.
- **Bounded subprocesses.** Template clones and fetches run under a timeout, and
  the child leads its own process group so a timeout tears down the whole tree
  rather than orphaning it.
- **No embedded credentials.** A template URL carrying a user, password, query
  string or fragment is refused with a message pointing at a git credential
  helper.
- **Working trees are written by one command only.** `aw` clones, fetches, sets
  git configuration and creates symlinks for skills. The exception is
  `aw fast-forward`, which runs only when asked and moves a clean checkout on
  its default branch with `git merge --ff-only`. Apart from that move, no
  command checks out, rebases or edits a file inside a managed repository.
- **Code safety.** `unsafe_code = "deny"` globally; clippy pedantic at warn;
  dependencies audited by `cargo-deny` in CI (advisories, licences, bans); every
  third-party GitHub action pinned to a commit hash.

## Assumptions

- The user reads a template before running `aw bootstrap` inside it, the same
  way they would read a repository before running its build. `aw` checks no
  signature, and nothing vouches for the commit the first `aw init` takes. A
  later `aw init` refuses a template that resolves to a different commit.
  Whether a template is trustworthy is the user's call.
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
