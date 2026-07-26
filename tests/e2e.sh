#!/bin/sh
# End-to-end test for `aw`.
#
# Builds a throwaway workspace against local bare repositories, bootstraps it
# twice, and asserts that the second run is a no-op. Requires `garden` on PATH.
set -eu

REPO="$(cd "$(dirname "$0")/.." && pwd)"
AW="$REPO/target/debug/aw"
WORK="$REPO/tmp/e2e"
FAILED=0

pass() { printf 'ok    %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1"; FAILED=1; }

assert() {
	if [ "$2" = "$3" ]; then pass "$1"; else
		fail "$1"
		printf '      expected: %s\n      actual:   %s\n' "$3" "$2"
	fi
}

[ -x "$AW" ] || { echo "build first: cargo build" >&2; exit 1; }

rm -rf "$WORK"
mkdir -p "$WORK/origins"

# --- fixtures ----------------------------------------------------------------
# A clonable workspace template exercises init without network access. The
# working tree and its bare clone are both used below.
git init -q --initial-branch=main "$WORK/seed-template"
mkdir -p "$WORK/seed-template/.github/workflows" "$WORK/seed-template/bin"
cat >"$WORK/seed-template/.gitignore" <<'EOF'
*
!.gitignore
!workspace.toml
!garden.yaml
!README.md
!.editorconfig
!conflicting-source.txt
!.seedignore
!CONTRACT.md
!.github/
!.github/**
!bin/
!bin/**
EOF
cat >"$WORK/seed-template/workspace.toml" <<'EOF'
[identity]
name = "Template User"
email = "template@example.com"

[workspace]
name = "CHANGEME"
EOF
cat >"$WORK/seed-template/garden.yaml" <<'EOF'
garden:
  includes:
    - .aw/trees.yaml
EOF
echo 'template readme' >"$WORK/seed-template/README.md"
echo 'root = true' >"$WORK/seed-template/.editorconfig"
echo '#!/bin/sh' >"$WORK/seed-template/bin/bootstrap"
chmod +x "$WORK/seed-template/bin/bootstrap"
echo 'name: template-ci' >"$WORK/seed-template/.github/workflows/ci.yml"
echo 'template contract' >"$WORK/seed-template/CONTRACT.md"
cat >"$WORK/seed-template/.seedignore" <<'EOF'
.github/
CONTRACT.md
.seedignore
EOF
git -C "$WORK/seed-template" add -A
git -C "$WORK/seed-template" -c user.name=t -c user.email=t@t commit -qm init
git -C "$WORK/seed-template" tag fixture-v1
git -C "$WORK/seed-template" tag release
git -C "$WORK/seed-template" switch -qc release
echo 'branch release' >"$WORK/seed-template/branch-release.txt"
git -C "$WORK/seed-template" add -f branch-release.txt
git -C "$WORK/seed-template" -c user.name=t -c user.email=t@t \
	commit -qm 'branch release'
RELEASE_SHA="$(git -C "$WORK/seed-template" rev-parse HEAD)"
git -C "$WORK/seed-template" switch -q main
git clone -q --bare "$WORK/seed-template" "$WORK/origins/template.git"
TEMPLATE_SHA="$(git -C "$WORK/seed-template" rev-parse fixture-v1^{commit})"
git clone -q "$WORK/seed-template" "$WORK/seed-conflicting-template"
echo 'must never reach the target' >"$WORK/seed-conflicting-template/conflicting-source.txt"
git -C "$WORK/seed-conflicting-template" add conflicting-source.txt
git -C "$WORK/seed-conflicting-template" -c user.name=t -c user.email=t@t \
	commit -qm 'conflicting source'
git clone -q --bare "$WORK/seed-conflicting-template" "$WORK/origins/conflicting-template.git"
git clone -q "$WORK/seed-template" "$WORK/seed-invalid-template"
printf '[workspace\n' >"$WORK/seed-invalid-template/workspace.toml"
git -C "$WORK/seed-invalid-template" add workspace.toml
git -C "$WORK/seed-invalid-template" -c user.name=t -c user.email=t@t \
	commit -qm 'invalid manifest'
git clone -q --bare "$WORK/seed-invalid-template" "$WORK/origins/invalid-template.git"

# Four member repositories. The shared skills repository is an ordinary member
# that happens to contain only skills, so it needs no special manifest section.
seed_skill() { # checkout, directory, description
	mkdir -p "$1/$2"
	printf -- '---\nname: %s\ndescription: %s\n---\n' \
		"$(basename "$2")" "$3" >"$1/$2/SKILL.md"
}

for name in alpha beta gamma skills; do
	git init -q --bare --initial-branch=main "$WORK/origins/$name.git"
	git init -q --initial-branch=main "$WORK/seed-$name"
	echo "$name" >"$WORK/seed-$name/file.txt"
	case "$name" in
	# alpha never opts in, so its skill must stay invisible.
	alpha) seed_skill "$WORK/seed-$name" .claude/skills/private-alpha "Never opted in." ;;
	# beta and gamma both expose `deploy`; only gamma prefixes, so both survive.
	beta) seed_skill "$WORK/seed-$name" .claude/skills/deploy "Beta deploy skill." ;;
	gamma) seed_skill "$WORK/seed-$name" .claude/skills/deploy "Gamma deploy skill." ;;
	# A dedicated skills repository uses the public/private split.
	skills) seed_skill "$WORK/seed-$name" public/release "Shared release skill." ;;
	esac
	git -C "$WORK/seed-$name" add -A
	git -C "$WORK/seed-$name" -c user.name=t -c user.email=t@t commit -qm init
	git -C "$WORK/seed-$name" push -q "$WORK/origins/$name.git" HEAD:main
done

# --- init --------------------------------------------------------------------
mkdir -p "$WORK/demo.workspace"
echo 'keep this readme' >"$WORK/demo.workspace/README.md"
"$AW" init --template "$WORK/origins/template.git@fixture-v1" \
	"$WORK/demo.workspace" --name demo >"$WORK/init.log" 2>&1
assert "init creates a manifest" "$([ -f "$WORK/demo.workspace/workspace.toml" ] && echo yes)" yes
assert "init creates a git repository" \
	"$([ -d "$WORK/demo.workspace/.git" ] && echo yes)" yes
assert "init detaches the template repository" \
	"$(git -C "$WORK/demo.workspace" remote | wc -l | tr -d ' ')" 0
assert "init makes no commit" \
	"$(git -C "$WORK/demo.workspace" rev-list --all --count)" 0
assert "init uses trunk as the initial branch" \
	"$(git -C "$WORK/demo.workspace" symbolic-ref --short HEAD)" trunk
assert "init records the workspace name" \
	"$(grep -c '^name = "demo"' "$WORK/demo.workspace/workspace.toml")" 1
assert "init records the template URL" \
	"$(grep -c "^url = \"$WORK/origins/template.git\"$" "$WORK/demo.workspace/workspace.toml")" 1
assert "init records the typed template ref" \
	"$(grep -c '^ref = "fixture-v1"$' "$WORK/demo.workspace/workspace.toml")" 1
assert "init records the resolved template SHA" \
	"$(grep -c "^sha = \"$TEMPLATE_SHA\"$" "$WORK/demo.workspace/workspace.toml")" 1
assert "init applies template identity" \
	"$(git -C "$WORK/demo.workspace" config --local user.email)" template@example.com
assert "init preserves files already present" \
	"$(cat "$WORK/demo.workspace/README.md")" 'keep this readme'
assert "init honours seedignore directory patterns" \
	"$([ -e "$WORK/demo.workspace/.github" ] && echo present || echo absent)" absent
assert "init honours seedignore file patterns" \
	"$([ -e "$WORK/demo.workspace/CONTRACT.md" ] && echo present || echo absent)" absent
assert "init removes seedignore itself" \
	"$([ -e "$WORK/demo.workspace/.seedignore" ] && echo present || echo absent)" absent
assert "init propagates formatting configuration" \
	"$([ -f "$WORK/demo.workspace/.editorconfig" ] && echo yes)" yes
assert "init preserves executable modes" \
	"$([ -x "$WORK/demo.workspace/bin/bootstrap" ] && echo yes)" yes

cp -a "$WORK/demo.workspace" "$WORK/demo-before-reinit"
"$AW" init --template "$WORK/origins/template.git@fixture-v1" \
	"$WORK/demo.workspace" --name demo >"$WORK/reinit.log" 2>&1
assert "same-source re-init leaves the target byte-for-byte unchanged" \
	"$(diff -qr "$WORK/demo-before-reinit" "$WORK/demo.workspace")" ""

if "$AW" init --template "$WORK/origins/conflicting-template.git@main" \
	"$WORK/demo.workspace" --name demo >"$WORK/conflicting-init.log" 2>&1; then
	fail "conflicting-source re-init fails"
else
	pass "conflicting-source re-init fails"
fi
assert "conflicting-source file does not reach the target" \
	"$([ -e "$WORK/demo.workspace/conflicting-source.txt" ] && echo present || echo absent)" absent
assert "conflicting-source re-init leaves the target byte-for-byte unchanged" \
	"$(diff -qr "$WORK/demo-before-reinit" "$WORK/demo.workspace")" ""

mkdir -p "$WORK/invalid.workspace"
echo 'keep this marker' >"$WORK/invalid.workspace/marker.txt"
cp -a "$WORK/invalid.workspace" "$WORK/invalid-before-init"
if "$AW" init --template "$WORK/origins/invalid-template.git@main" \
	"$WORK/invalid.workspace" --name invalid >"$WORK/invalid-init.log" 2>&1; then
	fail "invalid-manifest init fails"
else
	pass "invalid-manifest init fails"
fi
assert "invalid-manifest init leaves the target byte-for-byte unchanged" \
	"$(diff -qr "$WORK/invalid-before-init" "$WORK/invalid.workspace")" ""

"$AW" init --template "$WORK/seed-template" \
	"$WORK/working-tree.workspace" --name working >"$WORK/init-working.log" 2>&1
assert "init accepts a local working-tree template" \
	"$([ -f "$WORK/working-tree.workspace/workspace.toml" ] && echo yes)" yes
assert "working-tree init is detached" \
	"$(git -C "$WORK/working-tree.workspace" remote | wc -l | tr -d ' ')" 0
assert "working-tree init records an empty typed ref" \
	"$(grep -c '^ref = ""$' "$WORK/working-tree.workspace/workspace.toml")" 1

"$AW" init --template "$WORK/origins/template.git@main" \
	"$WORK/branch.workspace" --name branch >"$WORK/init-branch.log" 2>&1
assert "init resolves a branch ref" \
	"$(grep -c '^ref = "main"$' "$WORK/branch.workspace/workspace.toml")" 1
assert "branch ref records its resolved SHA" \
	"$(grep -c "^sha = \"$TEMPLATE_SHA\"$" "$WORK/branch.workspace/workspace.toml")" 1

"$AW" init --template "$WORK/origins/template.git@$TEMPLATE_SHA" \
	"$WORK/sha.workspace" --name sha >"$WORK/init-sha.log" 2>&1
assert "init resolves a SHA ref" \
	"$(grep -c "^ref = \"$TEMPLATE_SHA\"$" "$WORK/sha.workspace/workspace.toml")" 1
assert "SHA ref records its resolved SHA" \
	"$(grep -c "^sha = \"$TEMPLATE_SHA\"$" "$WORK/sha.workspace/workspace.toml")" 1

if "$AW" init --template "$WORK/origins/template.git@release" \
	"$WORK/ambiguous.workspace" --name ambiguous >"$WORK/init-ambiguous.log" 2>&1; then
	fail "ambiguous tag and branch ref is rejected"
else
	pass "ambiguous tag and branch ref is rejected"
fi

"$AW" init --template "$WORK/origins/template.git@refs/tags/release" \
	"$WORK/qualified-tag.workspace" --name qualified-tag \
	>"$WORK/init-qualified-tag.log" 2>&1
assert "qualified tag ref selects the tag commit" \
	"$([ -e "$WORK/qualified-tag.workspace/branch-release.txt" ] && echo branch || echo tag)" tag
assert "qualified tag records its resolved SHA" \
	"$(grep -c "^sha = \"$TEMPLATE_SHA\"$" \
		"$WORK/qualified-tag.workspace/workspace.toml")" 1

"$AW" init --template "$WORK/origins/template.git@refs/heads/release" \
	"$WORK/qualified-branch.workspace" --name qualified \
	>"$WORK/init-qualified-branch.log" 2>&1
assert "qualified branch ref fetches the requested commit" \
	"$(cat "$WORK/qualified-branch.workspace/branch-release.txt")" 'branch release'
assert "qualified branch records its resolved SHA" \
	"$(grep -c "^sha = \"$RELEASE_SHA\"$" \
		"$WORK/qualified-branch.workspace/workspace.toml")" 1

mkdir -p "$WORK/missing-ref.workspace"
echo 'keep this marker' >"$WORK/missing-ref.workspace/marker.txt"
cp -a "$WORK/missing-ref.workspace" "$WORK/missing-ref-before-init"
if "$AW" init --template "$WORK/origins/template.git@does-not-exist" \
	"$WORK/missing-ref.workspace" --name missing \
	>"$WORK/init-missing-ref.log" 2>&1; then
	fail "missing ref is rejected"
else
	pass "missing ref is rejected"
fi
assert "missing-ref init leaves the target byte-for-byte unchanged" \
	"$(diff -qr "$WORK/missing-ref-before-init" "$WORK/missing-ref.workspace")" ""

# --- manifest ----------------------------------------------------------------
cat >>"$WORK/demo.workspace/workspace.toml" <<EOF

[[repo]]
path = "alpha"
url = "$WORK/origins/alpha.git"
branch = "main"

[[repo]]
path = "beta"
url = "$WORK/origins/beta.git"
skills = true

[[repo]]
path = "gamma"
url = "$WORK/origins/gamma.git"
skills = true
skill-prefix = true

[[repo]]
path = "skills"
url = "$WORK/origins/skills.git"
skills = ["release"]
EOF

# A workspace-local skill that collides with the shared repository's name.
mkdir -p "$WORK/demo.workspace/.skills/release"
cat >"$WORK/demo.workspace/.skills/release/SKILL.md" <<'EOF'
---
name: release
description: Workspace-local release skill; must win over the shared one.
---
EOF

# --- bootstrap (first run) ---------------------------------------------------
"$AW" bootstrap "$WORK/demo.workspace" >"$WORK/boot1.log" 2>&1 ||
	{ cat "$WORK/boot1.log"; exit 1; }

for name in alpha beta gamma skills; do
	assert "$name cloned" "$([ -d "$WORK/demo.workspace/$name/.git" ] && echo yes)" yes
done
assert "identity applied to member repo" \
	"$(git -C "$WORK/demo.workspace/alpha" config --local user.email)" template@example.com
assert "identity applied to workspace repo" \
	"$(git -C "$WORK/demo.workspace" config --local user.email)" template@example.com
assert "branch honoured" \
	"$(git -C "$WORK/demo.workspace/alpha" rev-parse --abbrev-ref HEAD)" main

for dir in .claude/skills .agents/skills; do
	assert "link resolves in $dir" \
		"$(cat "$WORK/demo.workspace/$dir/release/SKILL.md" | grep -c 'must win')" 1
	assert "link in $dir is relative" \
		"$(readlink "$WORK/demo.workspace/$dir/release" | cut -c1-2)" ..
done
assert "shared skill is shadowed, not linked twice" \
	"$(grep -c 'shadowed  release' "$WORK/boot1.log")" 1
assert "member repo skill keeps its own name by default" \
	"$(grep -c 'Beta deploy skill' "$WORK/demo.workspace/.claude/skills/deploy/SKILL.md")" 1
assert "skill-prefix namespaces the repo that asks for it" \
	"$(grep -c 'Gamma deploy skill' "$WORK/demo.workspace/.claude/skills/gamma--deploy/SKILL.md")" 1
assert "opted-out repo contributes nothing" \
	"$(ls "$WORK/demo.workspace/.claude/skills" | grep -c 'private-alpha')" 0

# --- the deny-all invariant --------------------------------------------------
echo 'TOKEN=secret' >"$WORK/demo.workspace/leak.env"
assert "inner repos and secrets stay invisible" \
	"$(git -C "$WORK/demo.workspace" status --porcelain |
		grep -cE '^.. (alpha|beta|gamma|skills)/|leak\.env')" 0
git -C "$WORK/demo.workspace" add -A
assert "no gitlink is ever created" \
	"$(git -C "$WORK/demo.workspace" ls-files -s | grep -c 160000)" 0
git -C "$WORK/demo.workspace" reset -q

# --- bootstrap (second run must be a no-op) ----------------------------------
BEFORE="$(cd "$WORK/demo.workspace" && find . -path ./.git -prune -o -print | sort |
	while read -r p; do printf '%s %s\n' "$p" "$(git hash-object "$p" 2>/dev/null || echo dir)"; done)"
HEAD_BEFORE="$(git -C "$WORK/demo.workspace/alpha" rev-parse HEAD)"

"$AW" bootstrap "$WORK/demo.workspace" >"$WORK/boot2.log" 2>&1

AFTER="$(cd "$WORK/demo.workspace" && find . -path ./.git -prune -o -print | sort |
	while read -r p; do printf '%s %s\n' "$p" "$(git hash-object "$p" 2>/dev/null || echo dir)"; done)"

assert "second bootstrap changes nothing on disk" "$(
	[ "$BEFORE" = "$AFTER" ] && echo same
)" same
assert "second bootstrap regenerates nothing" \
	"$(grep -c 'trees     unchanged' "$WORK/boot2.log")" 1
assert "second bootstrap relinks nothing" \
	"$(grep -c ' 0 link(s) changed' "$WORK/boot2.log")" 2
assert "second bootstrap moves no HEAD" \
	"$(git -C "$WORK/demo.workspace/alpha" rev-parse HEAD)" "$HEAD_BEFORE"

# --- containment: a workspace manages nothing outside itself -----------------
"$AW" init --template "$WORK/seed-template" \
	"$WORK/escape.workspace" --name escape >/dev/null 2>&1
cat >>"$WORK/escape.workspace/workspace.toml" <<EOF

[[repo]]
path = "../outside"
url = "$WORK/origins/alpha.git"
EOF
if "$AW" bootstrap "$WORK/escape.workspace" >"$WORK/escape.log" 2>&1; then
	ESCAPE=allowed
else
	ESCAPE=rejected
fi
assert "a path escaping the workspace is rejected" "$ESCAPE" rejected
assert "the rejection names the cause" \
	"$(grep -c 'escapes the workspace' "$WORK/escape.log")" 1
assert "nothing was cloned outside the workspace" \
	"$([ -e "$WORK/outside" ] && echo yes || echo no)" no

# --- doctor ------------------------------------------------------------------
"$AW" doctor "$WORK/demo.workspace" >"$WORK/doctor.log" 2>&1 ||
	{ cat "$WORK/doctor.log"; fail "doctor exits non-zero on a healthy workspace"; }
assert "doctor reports no failures" "$(grep -c '^FAIL' "$WORK/doctor.log")" 0

ln -s ../../nowhere "$WORK/demo.workspace/.claude/skills/broken"
"$AW" doctor "$WORK/demo.workspace" >"$WORK/doctor2.log" 2>&1 || true
assert "doctor catches a dangling link" \
	"$(grep -c 'FAIL  skill links' "$WORK/doctor2.log")" 1

if [ "$FAILED" -eq 0 ]; then
	echo
	echo "e2e: all assertions passed"
else
	echo
	echo "e2e: FAILURES"
fi
exit "$FAILED"
