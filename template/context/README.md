# context/

Working context for this workspace: who is involved, the constraints the work
operates under, playbooks, decision precedents, and notes worth keeping between
sessions. Agents read this; humans maintain it.

**This directory is version-controlled.** Everything committed here reaches the
workspace remote, so its confidentiality is the remote's confidentiality — set
that expectation deliberately before writing anything sensitive. A workspace
pointed at a private remote can hold private material; one pointed anywhere
public cannot.

## What does not belong here

- Secrets of any kind: credentials, tokens, API keys, private keys, passwords
- `.env` files (hard-excluded by the workspace `.gitignore` regardless)
- Personal data beyond what the work legitimately requires
- Anything whose disclosure would be a breach rather than an inconvenience

Secrets live in the host's keychain or secret manager, referenced by name.
