# Workspace instructions

This is an agentic workspace: the root is a git repository tracking only the
workspace layer. Inner repositories are separate checkouts, invisible to it.

## Layout

| Path             | Purpose                                                |
| ---------------- | ------------------------------------------------------ |
| `workspace.toml` | Manifest — inner repositories, identity, skill opt-ins |
| `garden.yaml`    | Hand-editable garden commands; includes `.aw/`         |
| `context/`       | Working context for this workspace (tracked)           |
| `.skills/`       | Workspace-local skills; highest link precedence        |
| `bin/bootstrap`  | Provisioning entry point (delegates to `aw`)           |
| `.aw/`           | Generated from the manifest; untracked, never edited   |
| `tmp/`           | Scratch; untracked                                     |
| `<repo>/`        | Inner repositories; each its own git repo, untracked   |

## Rules

- Never commit inner-repo work from the workspace repo; commit inside the repo.
- `aw sync` (planned) will fetch and report. It never merges, pulls, or touches
  a working tree — do the same by hand until it lands.
- Secrets never enter `context/` or any tracked file.
- Throwaway files go in `tmp/`.

## Project notes

<!-- Fill in: what this workspace is for, scope, people, guardrails. -->
