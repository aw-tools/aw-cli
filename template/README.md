# Workspace template

Seed for a multi-repo agentic workspace.

A workspace is a git repository that tracks only its own thin layer — the manifest, agent
instructions, and working context. The inner repositories checked out inside it stay entirely
separate: the workspace `.gitignore` denies everything by default, so git never descends into them
and no gitlink is ever created. Clone the workspace anywhere, run one command, and the whole
constellation reassembles.

## Provisioning ritual

```sh
aw init <codename>.workspace   # instantiate this template, git init (no commit)
$EDITOR workspace.toml         # declare inner repos and skill opt-ins
bin/bootstrap                  # clone repos, apply config, link skills, report
```

The first commit is deliberately yours to make: review `git status --porcelain` and `git ls-files`
before it, because everything tracked here reaches the workspace remote.

## Day to day

| Command               | Effect                                                     |
| --------------------- | ---------------------------------------------------------- |
| `aw doctor`           | Environment and remote-reachability checks with remedies   |
| `aw status` (planned) | Per-repo state, config drift, skill-link health            |
| `aw sync` (planned)   | Fetch and report only — never merges, pulls, or checks out |

## See also

`aw` lives in the `agentic-workspace` repo.
