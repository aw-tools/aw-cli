# aw

Provision and report on reproducible multi-repository agentic workspaces.

A workspace is a git repository that tracks only its own thin layer: a manifest,
agent instructions and working context. The repositories you work on are cloned
inside it and stay separate, because the workspace ignores everything it does
not own. Clone the workspace on any machine, run one command, and the whole set
reassembles.

`aw` is the command line tool that does the provisioning and the reporting.
[The guide](https://github.com/aw-tools/agentic-workspace/blob/main/guide/01-what-this-is.md)
explains the whole toolkit for people.
[The contract](https://github.com/aw-tools/agentic-workspace/blob/main/SPEC.md)
says what any workspace must do, with a conformance suite beside it.

## Install

A Rust toolchain at 1.85 or later is required. Install from source:

```sh
cargo install --git https://github.com/aw-tools/aw-cli aw-cli
```

`aw` calls `git` and [`garden`](https://github.com/garden-rs/garden) at runtime,
so both must be on your `PATH`. `aw doctor` reports what is missing.

## Commands

| Command        | What it does                                                          |
| -------------- | --------------------------------------------------------------------- |
| `aw init`      | Create a new workspace from a cloned template                         |
| `aw bootstrap` | Clone missing repositories, converge configuration, link skills       |
| `aw doctor`    | Check the environment, remotes and link health                        |
| `aw status`    | Report repository presence, working-tree state and upstream distance  |
| `aw sync`      | Fetch the workspace layer and every managed repository, report change |
| `aw adopt`     | Add an existing checkout to the workspace manifest                    |

`aw --help` and `aw <command> --help` describe each option.

## Start a workspace

```sh
aw init my.workspace
cd my.workspace
aw bootstrap
```

`aw init` clones the
[workspace template](https://github.com/aw-tools/workspace.template) and lays it
out under the directory you name. Then `aw bootstrap` clones every repository
the manifest lists and links the agent skills. The guide's third chapter walks
through the first session.

## Contributing

Not accepting external contributions yet. See
[CONTRIBUTING.md](CONTRIBUTING.md). Security reports are welcome at any time;
see [SECURITY.md](SECURITY.md).

## Licence

Copyright 2026 Front Seed Labs Ltd.

Licensed under either [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option.
