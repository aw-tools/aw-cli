# Release process

This runbook says how to cut an `aw` release, check it, hotfix it, recover from
a failed cut and withdraw it.

## Overview

A release starts when you push a `v*` tag from `main`. The release workflow runs
the quality gate with `just ci` on the tagged commit, cross-compiles four
binaries, writes a `SHA256SUMS` file and publishes a GitHub release. The release
body is the matching section of `CHANGELOG.md`. The publish job waits at the
`release` GitHub Environment for the owner's approval, so push permission alone
cannot ship a release.

The four targets are `x86_64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`,
`aarch64-apple-darwin` and `x86_64-apple-darwin`. Each archive is named
`aw-<target>.tar.gz` and holds the binary, both licence files and the README.
The name carries no version, so the README's install snippet points at
`releases/latest/download/` and never goes stale.

## Prerequisites

The owner sets up the `release` Environment once. It has two halves, and both
are needed. Required reviewers gate the publish job, and a deployment rule says
which refs may reach the gate at all.

Add the owner as a required reviewer:

```sh
USER_ID=$(gh api /users/<your-handle> --jq .id)
gh api -X PUT /repos/aw-tools/aw-cli/environments/release \
  -F "reviewers[][type]=User" \
  -F "reviewers[][id]=$USER_ID"
```

Allow tags matching `v*` to deploy. Without this rule the first tag push fails
at once, because GitHub's default rejects tag deployments:

```sh
gh api -X PUT /repos/aw-tools/aw-cli/environments/release \
  -F "deployment_branch_policy[protected_branches]=false" \
  -F "deployment_branch_policy[custom_branch_policies]=true"
gh api -X POST /repos/aw-tools/aw-cli/environments/release/deployment-branch-policies \
  -F "name=v*" \
  -F "type=tag"
```

Check both halves:

```sh
gh api /repos/aw-tools/aw-cli/environments/release \
  --jq '{reviewers: .protection_rules[] | select(.type=="required_reviewers").reviewers[].reviewer.login,
         deployment_policy: .deployment_branch_policy}'
gh api /repos/aw-tools/aw-cli/environments/release/deployment-branch-policies \
  --jq '.branch_policies[] | {name, type}'
```

Without reviewers the publish job waits without end. Configure the reviewers,
then approve the pending deployment; no new tag is needed. Without the tag rule
the publish job fails at the gate. Add the rule, then rerun the failed job with
`gh run rerun <run-id> --failed`; this is safe because the run created nothing
on GitHub.

Never empty the reviewers list. It is the boundary between pushing code and
shipping a release.

On your machine you need `just`, `dprint` and `gh` installed and configured.
Start every release from a clean working tree on `main`.

## Versioning before 1.0

Before 1.0 the version rules are loose on purpose.

| Version shape        | When to use it                                       |
| -------------------- | ---------------------------------------------------- |
| `vX.Y.Z-rc.N`        | No known blockers, a last check before stable.       |
| `vX.Y.Z` (no suffix) | Stable. Before 1.0 a minor release may still break.  |
| `v1.0.0`             | A public stability promise the project has not made. |

Bump the minor version (`v0.X.0`) for a new capability or a change to what the
manifest, `workspace.toml`, accepts. Bump the patch version (`v0.0.Z`) for fixes
alone.

The first release is `v0.1.0`. Tag `v0.1.0-rc.1` first, to prove the pipeline on
a tag that nobody installs.

### What an upgrade may change

Within `0.x`, a release may add optional keys to `workspace.toml`. It never
removes or renames one. A removal or a rename is a minor release, and its
changelog entry says how to migrate. An upgrade never edits your manifest for
you.

## Cadence

Cut a release when there is something worth releasing. There is no fixed
schedule, because a schedule pushes out empty releases.

## Cutting a release

1. Decide the version from the table above.

2. Read the `[Unreleased]` block in `CHANGELOG.md`. Every pull request added its
   own line when it merged, so the block needs a read for order and sense, not
   new content. Do not run `just changelog` to rewrite it; that recipe prints
   candidate lines from the commit log and never edits the file.

3. Run the preparation recipe:

   ```sh
   just release-prep 0.1.0
   ```

   It bumps `Cargo.toml`, rotates the block into a dated `## [0.1.0]` section,
   formats both files and runs `just ci` against the result. It refuses to touch
   anything on an invalid version, a missing `[Unreleased]` heading, an existing
   section for that version or an empty block.

4. Open a pull request from a `ci/` branch. Branch names take one of six
   prefixes, `feat`, `fix`, `refactor`, `doc`, `ci` or `deps`, and release
   plumbing sits closest to CI:

   ```sh
   git checkout -b ci/release-0.1.0
   git commit -am 'chore(release): prepare 0.1.0'
   git push --set-upstream origin HEAD
   gh pr create --draft --title 'chore(release): prepare 0.1.0' --body-file body.md
   ```

   Wait for CI. The owner merges.

5. Tag from `main` and nowhere else. Sign the tag:

   ```sh
   git checkout main
   git pull origin main
   git tag -s v0.1.0 -m v0.1.0
   git push origin v0.1.0
   ```

6. Approve the publish job. The workflow runs its verify and build jobs on its
   own, then pauses at the `release` gate. Open the run under Actions and choose
   "Review deployments", then "Approve". Only a required reviewer can approve,
   in the browser or through the API:

   ```sh
   RUN_ID=<workflow-run-id>
   ENV_ID=$(gh api /repos/aw-tools/aw-cli/environments/release --jq .id)
   gh api -X POST "/repos/aw-tools/aw-cli/actions/runs/$RUN_ID/pending_deployments" \
     -F "environment_ids[]=$ENV_ID" \
     -F "state=approved" \
     -F "comment=approving v0.1.0"
   ```

## Post-release verification

Check the release from a clean directory on at least one machine. Name the tag,
not `latest`, which never points at a prerelease:

```sh
base=https://github.com/aw-tools/aw-cli/releases/download/v0.1.0
curl -LO "$base/aw-x86_64-unknown-linux-gnu.tar.gz" -O "$base/SHA256SUMS"
sha256sum -c SHA256SUMS --ignore-missing
tar xzf aw-x86_64-unknown-linux-gnu.tar.gz
./aw --version
./aw doctor <an existing workspace>
```

The checksum matches, the binary runs and the version string equals the tag.

## Hotfix path

A hotfix follows the same merge-then-tag flow, with one constraint: the branch
starts at the tagged commit, not at the head of `main`.

```sh
git checkout v0.1.0
git checkout -b fix/critical-thing
# fix and commit
git push --set-upstream origin HEAD
gh pr create --draft --title 'fix: critical thing' --body-file body.md
# the owner merges to main
git checkout main && git pull
just release-prep 0.1.1
# move unrelated lines back under [Unreleased]
git commit -am 'chore(release): prepare 0.1.1'
# pull request, merge and tag from main as usual
```

For a hotfix, `just release-prep` rotates the whole `[Unreleased]` block,
including lines that are not part of the fix. Move those lines back under a
fresh `[Unreleased]` heading before you commit.

Never tag a hotfix branch, so that every released commit is reachable from
`main`. If the fix conflicts with `main` beyond a clean cherry-pick, cut a minor
release instead of forcing a hotfix.

## Prerelease promotion

When a release candidate becomes stable, keep the candidate tags and releases as
they are. They are the public record of the path to the release. GitHub leaves
prereleases out of `latest`, so the pointer moves to the stable release on its
own and the install instructions keep resolving.

## Failure modes

### The tag points at a commit that is not on main

The workflow builds the commit the tag names, not `main`. Delete the release and
the tag as described under "The release already exists", then tag again from
`main` with the next version.

### Verify fails on the tagged commit

`just ci` failed on the tagged commit. Do not tag the same version again. Fix it
in a pull request on `main`, bump the patch version and cut again. Remove the
failed tag:

```sh
git push origin :refs/tags/v0.1.0
git tag -d v0.1.0
```

### One build target fails

Two ways out, with the trade-off named:

- Ship without that target. Edit the matrix in a follow-up commit, state the gap
  in the changelog, bump the patch version and cut again. Users on that target
  wait for the next release.
- Block the release. Delete the partial release and tag, fix the build in a pull
  request, cut again at the next patch version.

If the failing target is `aarch64-apple-darwin`, block; Apple silicon is a
common place to run `aw`. Otherwise shipping without the target is acceptable.

### The release already exists

A release with that tag exists from an earlier run. Delete both, bump the
version and cut again:

```sh
gh release delete v0.1.0 --cleanup-tag --yes
```

If your `gh` lacks `--cleanup-tag`:

```sh
gh release delete v0.1.0 --yes
git push origin :refs/tags/v0.1.0
git tag -d v0.1.0
```

Never reuse a version. Retagging without thought is how broken binaries ship.

### Rerun or retag

Do not rerun a failed release workflow, in the browser or with
`gh run rerun --failed`. The first run may have created state on GitHub that the
rerun collides with. Delete the release and the tag, bump the version and cut
again.

Rerunning is safe for the CI workflow and unsafe for the release workflow. The
one exception is a run that failed at the gate before the publish job ran, as
described under Prerequisites.

## Withdrawing a release

Withdraw a release when a defect turns up after publication. The archives stay
on the release page so the checksums remain on record, and `latest` skips the
release.

```sh
# 1. Mark it a prerelease so latest moves back to the last good release.
gh release edit v0.1.0 --prerelease

# 2. Say what is wrong and where the fix is.
$EDITOR notice.md
gh release edit v0.1.0 --notes-file notice.md

# 3. Cut the fix at the next patch version, following the steps above.
```

The install instructions point at `latest`, so they need no change for a
withdrawal. Edit `CHANGELOG.md` after the fact only when the defect put data or
security at risk.

## Auditability

Every write the workflow makes uses the repository's own token and lands in the
repository audit log. Match a release's creation time against the workflow run
that made it.

## Cross-compile workarounds

None are known. CI cross-compiles all four targets with `cargo zigbuild` on
every pull request, and the release workflow reuses those steps. When a target
needs a workaround, record it here with the failure it answers and the condition
for removing it.
