#!/bin/sh
# End-to-end test for `aw`.
#
# Builds isolated throwaway workspaces against shared local bare repositories.
# Requires `garden` on PATH.
set -eu

REPO="$(cd "$(dirname "$0")/.." && pwd)"
AW="$REPO/target/debug/aw"
WORK="$REPO/tmp/e2e"
FAILED=0

pass() { printf 'ok    %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1"; CASE_FAILED=1; }

assert() {
	if [ "$2" = "$3" ]; then pass "$1"; else
		fail "$1"
		printf '      expected: %s\n      actual:   %s\n' "$3" "$2"
	fi
}

run_case() { # label, function
	(
		set -eu
		CASE_FAILED=0
		WORK="$FIXTURES/cases/$1"
		rm -rf "$WORK"
		mkdir -p "$WORK"
		ln -s "$FIXTURES/origins" "$WORK/origins"
		for fixture in seed-template seed-conflicting-template seed-invalid-template; do
			ln -s "$FIXTURES/$fixture" "$WORK/$fixture"
		done
		"$2"
		[ "$CASE_FAILED" -eq 0 ]
	)

	case "$?" in
	0) pass "case $1" ;;
	*)
		printf 'FAIL  case %s\n' "$1"
		FAILED=1
		;;
	esac
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
!.githooks/
!.githooks/**
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
mkdir -p "$WORK/seed-template/.githooks"
cat >"$WORK/seed-template/.githooks/pre-commit" <<'EOF'
#!/bin/sh
exit 0
EOF
chmod +x "$WORK/seed-template/.githooks/pre-commit"
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

for name in ahead behind divergent dirty missing; do
	git init -q --bare --initial-branch=main "$WORK/origins/$name.git"
	git init -q --initial-branch=main "$WORK/seed-$name"
	echo "$name base" >"$WORK/seed-$name/file.txt"
	git -C "$WORK/seed-$name" add file.txt
	GIT_AUTHOR_DATE=2000-01-01T00:00:00Z \
		GIT_COMMITTER_DATE=2000-01-01T00:00:00Z \
		git -C "$WORK/seed-$name" -c user.name=t -c user.email=t@t \
			commit -qm base
	echo "$name origin" >>"$WORK/seed-$name/file.txt"
	git -C "$WORK/seed-$name" add file.txt
	GIT_AUTHOR_DATE=2000-01-01T00:00:01Z \
		GIT_COMMITTER_DATE=2000-01-01T00:00:01Z \
		git -C "$WORK/seed-$name" -c user.name=t -c user.email=t@t \
			commit -qm origin
	git -C "$WORK/seed-$name" push -q "$WORK/origins/$name.git" HEAD:main
done

FIXTURES="$WORK"

init_demo_workspace() {
	mkdir -p "$WORK/demo.workspace"
	echo 'keep this readme' >"$WORK/demo.workspace/README.md"
	"$AW" init --template "$WORK/origins/template.git@fixture-v1" \
		"$WORK/demo.workspace" --name demo >"$WORK/init.log" 2>&1
}

prepare_demo_workspace() {
	init_demo_workspace
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

	mkdir -p "$WORK/demo.workspace/.skills/release"
	cat >"$WORK/demo.workspace/.skills/release/SKILL.md" <<'EOF'
---
name: release
description: Workspace-local release skill; must win over the shared one.
---
EOF
}

setup_status_fixtures() {
	"$AW" init --template "$WORK/seed-template" \
		"$WORK/status.workspace" --name status >"$WORK/status-init.log" 2>&1
	git -C "$WORK/status.workspace" config --local core.hooksPath .githooks
	cat >>"$WORK/status.workspace/workspace.toml" <<EOF

[[repo]]
path = "ahead"
url = "$WORK/origins/ahead.git"

[[repo]]
path = "behind"
url = "$WORK/origins/behind.git"

[[repo]]
path = "divergent"
url = "$WORK/origins/divergent.git"

[[repo]]
path = "dirty"
url = "$WORK/origins/dirty.git"

[[repo]]
path = "missing"
url = "$WORK/origins/missing.git"
EOF

	for name in ahead behind divergent dirty; do
		git clone -q "$WORK/origins/$name.git" "$WORK/status.workspace/$name"
		git -C "$WORK/status.workspace/$name" config --local user.name "Template User"
		git -C "$WORK/status.workspace/$name" config --local user.email template@example.com
	done

	git -C "$WORK/status.workspace/dirty" branch local-upstream origin/main
	git -C "$WORK/status.workspace/dirty" branch --set-upstream-to=local-upstream main \
		>/dev/null

	git -C "$WORK/status.workspace/behind" update-ref -d refs/remotes/origin/main
	git -C "$WORK/status.workspace/behind" fetch -q origin \
		main:refs/remotes/origin/main

	echo 'ahead local' >>"$WORK/status.workspace/ahead/file.txt"
	git -C "$WORK/status.workspace/ahead" add file.txt
	git -C "$WORK/status.workspace/ahead" -c user.name=t -c user.email=t@t \
		commit -qm local

	git -C "$WORK/status.workspace/behind" reset -q --hard HEAD^

	git -C "$WORK/status.workspace/divergent" reset -q --hard HEAD^
	echo 'divergent local' >>"$WORK/status.workspace/divergent/file.txt"
	git -C "$WORK/status.workspace/divergent" add file.txt
	git -C "$WORK/status.workspace/divergent" -c user.name=t -c user.email=t@t \
		commit -qm local
	rm -f "$WORK/status.workspace/divergent/.git/logs/HEAD"
	rm -f "$WORK/status.workspace/divergent/.git/logs/refs/remotes/origin/main"

	echo 'dirty local' >>"$WORK/status.workspace/dirty/file.txt"
}

# --- init --------------------------------------------------------------------
case_init() {
init_demo_workspace
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
}

# --- bootstrap (first run) ---------------------------------------------------
case_bootstrap() {
prepare_demo_workspace
"$AW" bootstrap "$WORK/demo.workspace" >"$WORK/boot1.log" 2>&1 ||
	{ cat "$WORK/boot1.log"; exit 1; }

for name in alpha beta gamma skills; do
	assert "$name cloned" "$([ -d "$WORK/demo.workspace/$name/.git" ] && echo yes)" yes
done
assert "identity applied to member repo" \
	"$(git -C "$WORK/demo.workspace/alpha" config --local user.email)" template@example.com
assert "identity applied to workspace repo" \
	"$(git -C "$WORK/demo.workspace" config --local user.email)" template@example.com
assert "bootstrap activates the workspace hook" \
	"$(git -C "$WORK/demo.workspace" config --local core.hooksPath 2>/dev/null || true)" .githooks
assert "bootstrap reports workspace hook activation" \
	"$(grep -c 'hooks.*core\.hooksPath.*\.githooks' "$WORK/boot1.log")" 1
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
}

# --- containment: a workspace manages nothing outside itself -----------------
case_containment() {
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
}

# --- adopt -------------------------------------------------------------------
case_adopt() {
"$AW" init --template "$WORK/seed-template" \
	"$WORK/adopt.workspace" --name adopt >/dev/null 2>&1
cat >>"$WORK/adopt.workspace/workspace.toml" <<EOF

# existing repository comment
[[repo]]
path = "existing"
url = "$WORK/origins/beta.git"

# Keep this commented example after the live repository entries.
# [[repo]]
# path = "example"
# url = "git@example.invalid:owner/example.git"
EOF
git clone -q "$WORK/origins/alpha.git" "$WORK/adopt.workspace/members/alpha"
COMMITS_BEFORE="$(git -C "$WORK/adopt.workspace" rev-list --all --count)"

(
	cd "$WORK/adopt.workspace/members/alpha"
	"$AW" adopt . >"$WORK/adopt.log" 2>&1
)
assert "adopt stores a workspace-relative path" \
	"$(grep -c '^path = "members/alpha"$' "$WORK/adopt.workspace/workspace.toml")" 1
assert "adopt derives the origin URL" \
	"$(grep -c "^url = \"$WORK/origins/alpha.git\"$" \
		"$WORK/adopt.workspace/workspace.toml")" 1
assert "adopt records the checked-out branch" \
	"$(grep -c '^branch = "main"$' "$WORK/adopt.workspace/workspace.toml")" 1
assert "adopt reports the added entry" \
	"$(grep -c 'members/alpha.*main' "$WORK/adopt.log")" 1
assert "adopt identifies the changed workspace manifest" \
	"$(grep -Fc "$WORK/adopt.workspace/workspace.toml" "$WORK/adopt.log")" 1
assert "adopt identifies the workspace repository" \
	"$(grep -Fc "workspace repository $WORK/adopt.workspace" "$WORK/adopt.log")" 1
for comment in \
	'# existing repository comment' \
	'# Keep this commented example after the live repository entries.' \
	'# [[repo]]' \
	'# path = "example"' \
	'# url = "git@example.invalid:owner/example.git"'
do
	assert "adopt preserves comment: $comment" \
		"$(grep -Fxc "$comment" "$WORK/adopt.workspace/workspace.toml")" 1
done
ADOPT_LINE="$(grep -n '^path = "members/alpha"$' \
	"$WORK/adopt.workspace/workspace.toml" | cut -d: -f1)"
EXAMPLE_LINE="$(grep -n '^# path = "example"$' \
	"$WORK/adopt.workspace/workspace.toml" | cut -d: -f1)"
assert "adopt inserts before trailing commented examples" \
	"$([ "$ADOPT_LINE" -lt "$EXAMPLE_LINE" ] && echo yes || echo no)" yes
assert "adopt makes no commit" \
	"$(git -C "$WORK/adopt.workspace" rev-list --all --count)" "$COMMITS_BEFORE"

cp "$WORK/adopt.workspace/workspace.toml" "$WORK/adopt-once.toml"
if (
	cd "$WORK/adopt.workspace"
	"$AW" adopt members/alpha >"$WORK/adopt-duplicate.log" 2>&1
); then
	fail "adopting the same checkout twice is rejected"
else
	pass "adopting the same checkout twice is rejected"
fi
assert "duplicate adoption names the cause" \
	"$(grep -c 'already.*manifest' "$WORK/adopt-duplicate.log")" 1
assert "duplicate adoption leaves the manifest unchanged" \
	"$(cmp -s "$WORK/adopt-once.toml" \
		"$WORK/adopt.workspace/workspace.toml" && echo yes || echo no)" yes

git clone -q "$WORK/origins/beta.git" "$WORK/adopt.workspace/locked"
mkdir "$WORK/adopt.workspace/.aw-adopt.lock"
cp "$WORK/adopt.workspace/workspace.toml" "$WORK/adopt-locked-before.toml"
if (
	cd "$WORK/adopt.workspace"
	"$AW" adopt locked >"$WORK/adopt-locked.log" 2>&1
); then
	fail "adopt lock rejects a concurrent adoption"
else
	pass "adopt lock rejects a concurrent adoption"
fi
rmdir "$WORK/adopt.workspace/.aw-adopt.lock"
assert "lock rejection names the concurrent operation" \
	"$(grep -c 'another `aw adopt` is running' "$WORK/adopt-locked.log")" 1
assert "lock rejection leaves the manifest unchanged" \
	"$(cmp -s "$WORK/adopt-locked-before.toml" \
		"$WORK/adopt.workspace/workspace.toml" && echo yes || echo no)" yes

git clone -q "$WORK/origins/beta.git" \
	"$WORK/adopt.workspace/members/equivalent"
cat >>"$WORK/adopt.workspace/workspace.toml" <<EOF

[[repo]]
path = "members/./equivalent"
url = "$WORK/origins/beta.git"
EOF
cp "$WORK/adopt.workspace/workspace.toml" "$WORK/adopt-equivalent-before.toml"
if (
	cd "$WORK/adopt.workspace"
	"$AW" adopt members/equivalent >"$WORK/adopt-equivalent.log" 2>&1
); then
	fail "adopting an equivalently-spelled checkout twice is rejected"
else
	pass "adopting an equivalently-spelled checkout twice is rejected"
fi
assert "equivalent duplicate adoption names the cause" \
	"$(grep -c 'already.*manifest' "$WORK/adopt-equivalent.log")" 1
assert "equivalent duplicate adoption leaves the manifest unchanged" \
	"$(cmp -s "$WORK/adopt-equivalent-before.toml" \
		"$WORK/adopt.workspace/workspace.toml" && echo yes || echo no)" yes

git clone -q "$WORK/origins/gamma.git" "$WORK/adopt.workspace/members.alpha"
if (
	cd "$WORK/adopt.workspace"
	"$AW" adopt members.alpha >"$WORK/adopt-collision.log" 2>&1
); then
	fail "adopting a colliding tree name is rejected"
else
	pass "adopting a colliding tree name is rejected"
fi
assert "tree-name collision names the cause" \
	"$(grep -c 'collides.*tree name' "$WORK/adopt-collision.log")" 1

git clone -q "$WORK/origins/beta.git" "$WORK/outside"
if (
	cd "$WORK/adopt.workspace"
	"$AW" adopt "$WORK/outside" >"$WORK/adopt-outside.log" 2>&1
); then
	fail "adopting a checkout outside the workspace is rejected"
else
	pass "adopting a checkout outside the workspace is rejected"
fi
assert "outside-checkout rejection names the cause" \
	"$(grep -c 'outside the workspace' "$WORK/adopt-outside.log")" 1

git clone -q "$WORK/origins/gamma.git" "$WORK/adopt.workspace/linked-base"
git -C "$WORK/adopt.workspace/linked-base" worktree add -qb linked-branch \
	"$WORK/adopt.workspace/members/linked"
(
	cd "$WORK/adopt.workspace"
	"$AW" adopt members/linked >"$WORK/adopt-linked.log" 2>&1
)
assert "adopt accepts linked worktree metadata inside the workspace" \
	"$(grep -c '^path = "members/linked"$' \
		"$WORK/adopt.workspace/workspace.toml")" 1
assert "adopt records a linked worktree branch" \
	"$(grep -c '^branch = "linked-branch"$' \
		"$WORK/adopt.workspace/workspace.toml")" 1

git clone -q --separate-git-dir "$WORK/outside-metadata.git" \
	"$WORK/origins/beta.git" "$WORK/adopt.workspace/outside-metadata"
if (
	cd "$WORK/adopt.workspace"
	"$AW" adopt outside-metadata >"$WORK/adopt-outside-metadata.log" 2>&1
); then
	fail "adopting a checkout with outside git metadata is rejected"
else
	pass "adopting a checkout with outside git metadata is rejected"
fi
assert "outside-metadata rejection names the cause" \
	"$(grep -c 'git directory.*outside the workspace' \
		"$WORK/adopt-outside-metadata.log")" 1

mkdir "$WORK/adopt.workspace/not-a-repo"
if (
	cd "$WORK/adopt.workspace"
	"$AW" adopt not-a-repo >"$WORK/adopt-not-repo.log" 2>&1
); then
	fail "adopting a non-repository is rejected"
else
	pass "adopting a non-repository is rejected"
fi
assert "non-repository rejection names the cause" \
	"$(grep -c 'not a git repository' "$WORK/adopt-not-repo.log")" 1

git clone -q "$WORK/origins/beta.git" "$WORK/adopt.workspace/no-origin"
git -C "$WORK/adopt.workspace/no-origin" remote remove origin
if (
	cd "$WORK/adopt.workspace"
	"$AW" adopt no-origin >"$WORK/adopt-no-origin.log" 2>&1
); then
	fail "adopting a checkout without origin is rejected"
else
	pass "adopting a checkout without origin is rejected"
fi
assert "missing-origin rejection names the cause" \
	"$(grep -c 'no origin remote' "$WORK/adopt-no-origin.log")" 1

"$AW" init --template "$WORK/seed-template" \
	"$WORK/adopt-symlink.workspace" --name adopt-symlink >/dev/null 2>&1
git clone -q "$WORK/origins/alpha.git" \
	"$WORK/adopt-symlink.workspace/member"
cp "$WORK/adopt-symlink.workspace/workspace.toml" \
	"$WORK/external-workspace.toml"
cp "$WORK/external-workspace.toml" "$WORK/external-workspace-before.toml"
rm "$WORK/adopt-symlink.workspace/workspace.toml"
ln -s "$WORK/external-workspace.toml" \
	"$WORK/adopt-symlink.workspace/workspace.toml"
if (
	cd "$WORK/adopt-symlink.workspace"
	"$AW" adopt member >"$WORK/adopt-symlink.log" 2>&1
); then
	fail "adopt rejects a symlinked workspace manifest"
else
	pass "adopt rejects a symlinked workspace manifest"
fi
assert "symlink rejection requires a regular manifest" \
	"$(grep -c 'workspace.toml must be a regular file' \
		"$WORK/adopt-symlink.log")" 1
assert "symlink rejection leaves the external manifest unchanged" \
	"$(cmp -s "$WORK/external-workspace-before.toml" \
		"$WORK/external-workspace.toml" && echo yes || echo no)" yes
}

# --- doctor ------------------------------------------------------------------
case_doctor() {
prepare_demo_workspace
"$AW" bootstrap "$WORK/demo.workspace" >"$WORK/boot1.log" 2>&1
"$AW" init --template "$WORK/seed-template" \
	"$WORK/working-tree.workspace" --name working >"$WORK/init-working.log" 2>&1
"$AW" init --template "$WORK/origins/template.git@main" \
	"$WORK/branch.workspace" --name branch >"$WORK/init-branch.log" 2>&1

"$AW" doctor "$WORK/demo.workspace" >"$WORK/doctor.log" 2>&1 ||
	{ cat "$WORK/doctor.log"; fail "doctor exits non-zero on a healthy workspace"; }
assert "doctor reports no failures" "$(grep -c '^FAIL' "$WORK/doctor.log")" 0
assert "doctor passes the workspace hook check" \
	"$(grep -c '^PASS  pre-commit hook.*core\.hooksPath = \.githooks' "$WORK/doctor.log")" 1

git -C "$WORK/working-tree.workspace" config --local core.hooksPath .other
"$AW" bootstrap "$WORK/working-tree.workspace" >"$WORK/custom-hook-bootstrap.log" 2>&1
assert "bootstrap preserves a custom workspace hook path" \
	"$(git -C "$WORK/working-tree.workspace" config --local core.hooksPath)" .other
assert "bootstrap reports the custom workspace hook path unchanged" \
	"$(grep -c 'hooks.*core\.hooksPath.*unchanged.*\.other' \
		"$WORK/custom-hook-bootstrap.log")" 1
if "$AW" doctor "$WORK/working-tree.workspace" >"$WORK/custom-hook-doctor.log" 2>&1; then
	fail "doctor fails when the workspace hook path is wrong"
else
	pass "doctor fails when the workspace hook path is wrong"
fi
assert "doctor reports the missing workspace hook" \
	"$(grep -c '^FAIL  pre-commit hook' "$WORK/custom-hook-doctor.log")" 1
assert "doctor gives an actionable workspace hook remedy" \
	"$(grep -c 'unset core\.hooksPath, then run `aw bootstrap`' \
		"$WORK/custom-hook-doctor.log")" 1

git -C "$WORK/working-tree.workspace" config --local core.hooksPath '.githooks '
if "$AW" doctor "$WORK/working-tree.workspace" >"$WORK/spaced-hook-doctor.log" 2>&1; then
	fail "doctor rejects a hook path with semantic trailing whitespace"
else
	pass "doctor rejects a hook path with semantic trailing whitespace"
fi
assert "doctor preserves semantic hook-path whitespace in its report" \
	"$(grep -c 'core\.hooksPath = \.githooks $' "$WORK/spaced-hook-doctor.log")" 1

rm -rf "$WORK/branch.workspace/.githooks"
"$AW" bootstrap "$WORK/branch.workspace" >"$WORK/no-hook-bootstrap.log" 2>&1
assert "bootstrap skips hook activation when the template has no hook" \
	"$(grep -c '^hooks ' "$WORK/no-hook-bootstrap.log")" 0
"$AW" doctor "$WORK/branch.workspace" >"$WORK/no-hook-doctor.log" 2>&1
assert "doctor skips the hook check when the template has no hook" \
	"$(grep -c 'pre-commit hook' "$WORK/no-hook-doctor.log")" 0

ln -s ../../nowhere "$WORK/demo.workspace/.claude/skills/broken"
"$AW" doctor "$WORK/demo.workspace" >"$WORK/doctor2.log" 2>&1 || true
assert "doctor catches a dangling link" \
	"$(grep -c 'FAIL  skill links' "$WORK/doctor2.log")" 1
}

# --- reusable status fixtures -------------------------------------------------
case_status_fixtures() {
setup_status_fixtures

assert "ahead fixture is ahead" \
	"$(git -C "$WORK/status.workspace/ahead" rev-list --left-right --count \
		HEAD...@{upstream} | tr '\t' ' ')" "1 0"
assert "behind fixture is behind" \
	"$(git -C "$WORK/status.workspace/behind" rev-list --left-right --count \
		HEAD...@{upstream} | tr '\t' ' ')" "0 1"
assert "divergent fixture is ahead and behind" \
	"$(git -C "$WORK/status.workspace/divergent" rev-list --left-right --count \
		HEAD...@{upstream} | tr '\t' ' ')" "1 1"
assert "dirty fixture has worktree changes" \
	"$(git -C "$WORK/status.workspace/dirty" status --porcelain | wc -l | tr -d ' ')" 1
assert "missing fixture is not cloned" \
	"$([ -e "$WORK/status.workspace/missing" ] && echo present || echo absent)" absent
}

# --- status ------------------------------------------------------------------
case_status() {
setup_status_fixtures

REAL_GIT="$(command -v git)"
mkdir -p "$WORK/fake-bin"
cat >"$WORK/fake-bin/git" <<EOF
#!/bin/sh
printf '%s\n' "\$*" >>"\$GIT_LOG"
exec "$REAL_GIT" "\$@"
EOF
chmod +x "$WORK/fake-bin/git"

GIT_LOG="$WORK/status-git.log" PATH="$WORK/fake-bin:$PATH" \
	"$AW" status "$WORK/status.workspace" >"$WORK/status-human.log" \
	2>"$WORK/status-human.err"
assert "status reports the workspace repository present" \
	"$(grep -c '^workspace[[:space:]]*present$' "$WORK/status-human.log")" 1
assert "status reports a member repository present" \
	"$(grep -c '^dirty[[:space:]]*present$' "$WORK/status-human.log")" 1
assert "status reports an absent member without probing it" \
	"$(grep -c '^missing[[:space:]]*declared, not present$' "$WORK/status-human.log")" 1
assert "status reports a dirty member working tree" \
	"$(grep -c '^dirty[[:space:]]*dirty$' "$WORK/status-human.log")" 1
assert "status reports a clean member working tree" \
	"$(grep -c '^ahead[[:space:]]*clean$' "$WORK/status-human.log")" 1
assert "status reports no working tree for an absent member" \
	"$(grep -c '^missing[[:space:]]*not present$' "$WORK/status-human.log")" 1
assert "status reports a repository ahead with clone age inline" \
	"$(grep -Ec '^ahead[[:space:]]+ahead 1, behind 0 \(cloned [0-9]+s ago\)$' \
		"$WORK/status-human.log")" 1
assert "status reports a repository behind with fetch age inline" \
	"$(grep -Ec '^behind[[:space:]]+ahead 0, behind 1 \(fetched [0-9]+s ago\)$' \
		"$WORK/status-human.log")" 1
assert "status reports a divergent repository with the unknown-age sync hint" \
	"$(grep -c '^divergent[[:space:]]*ahead 1, behind 1 (age unknown; run `aw sync` to refresh)$' \
		"$WORK/status-human.log")" 1
assert "status reports a clean zero-distance local upstream as cloned" \
	"$(grep -Ec '^dirty[[:space:]]+ahead 0, behind 0 \(cloned [0-9]+s ago\)$' \
		"$WORK/status-human.log")" 1
assert "status reports a repository without an upstream" \
	"$(grep -c '^workspace[[:space:]]*no upstream$' "$WORK/status-human.log")" 1
assert "status skips an absent member in the ahead-behind section" \
	"$(awk '/^ahead\/behind$/ { section = 1; next }
		section && /^missing[[:space:]]/ { count++ }
		END { print count + 0 }' "$WORK/status-human.log")" 0
assert "status performs no remote or fetch command" \
	"$(grep -Ec '(^| )(ls-remote|fetch)( |$)' "$WORK/status-git.log" || true)" 0
assert "status writes no diagnostics for ordinary findings" \
	"$(wc -c <"$WORK/status-human.err" | tr -d ' ')" 0

"$AW" status --json "$WORK/status.workspace" >"$WORK/status.json"
sed -n '/"name": "ahead_behind"/,$p' "$WORK/status.json" \
	>"$WORK/status-ahead-behind.json"
assert "status JSON names the workspace" \
	"$(grep -c '"workspace": "status"' "$WORK/status.json")" 1
assert "status JSON includes the presence section" \
	"$(grep -c '"name": "presence"' "$WORK/status.json")" 1
assert "status JSON includes the working-tree section" \
	"$(grep -c '"name": "working_tree"' "$WORK/status.json")" 1
assert "status JSON includes the ahead-behind section" \
	"$(grep -c '"name": "ahead_behind"' "$WORK/status.json")" 1
assert "status JSON reports an absent member" \
	"$(grep -A2 '"repository": "missing"' "$WORK/status.json" |
		grep -c '"state": "absent"')" 1
assert "status JSON reports no working tree for an absent member" \
	"$(grep -A2 '"repository": "missing"' "$WORK/status.json" |
		grep -c '"state": "not_present"')" 1
assert "status JSON reports a dirty member working tree" \
	"$(grep -A2 '"repository": "dirty"' "$WORK/status.json" |
		grep -c '"state": "dirty"')" 1
assert "status JSON reports a clean member working tree" \
	"$(grep -A2 '"repository": "ahead"' "$WORK/status.json" |
		grep -c '"state": "clean"')" 1
assert "status JSON reports ahead and behind counts" \
	"$(grep -B10 '"repository": "divergent"' "$WORK/status-ahead-behind.json" |
		grep -Ec '"ahead": 1|\"behind\": 1')" 2
assert "status JSON reports clone provenance with a raw timestamp" \
	"$(grep -B10 '"repository": "ahead"' "$WORK/status-ahead-behind.json" |
		grep -Ec '"kind": "cloned"|\"timestamp\": [0-9]+')" 2
assert "status JSON reports fetch provenance with a raw timestamp" \
	"$(grep -B10 '"repository": "behind"' "$WORK/status-ahead-behind.json" |
		grep -Ec '"kind": "fetched"|\"timestamp\": [0-9]+')" 2
assert "status JSON reports a zero-distance local upstream as cloned" \
	"$(grep -B10 '"repository": "dirty"' "$WORK/status-ahead-behind.json" |
		grep -Ec '"ahead": 0|\"behind\": 0|\"kind\": \"cloned\"')" 3
assert "status JSON reports unknown provenance without a timestamp" \
	"$(grep -B10 '"repository": "divergent"' "$WORK/status-ahead-behind.json" |
		grep -c '"kind": "unknown"')" 1
assert "status JSON reports a repository without an upstream" \
	"$(grep -B4 '"repository": "workspace"' "$WORK/status-ahead-behind.json" |
		grep -c '"state": "no_upstream"')" 1
assert "status JSON replaces the human table" \
	"$(grep -c '^workspace[[:space:]]' "$WORK/status.json" || true)" 0
assert "status help labels JSON unstable during incubation" \
	"$("$AW" status --help | grep -ci 'unstable during incubation')" 1

git -C "$WORK/status.workspace/dirty" checkout -q --detach
"$AW" status "$WORK/status.workspace" >"$WORK/status-detached.log"
assert "status reports detached HEAD as no upstream" \
	"$(grep -c '^dirty[[:space:]]*no upstream$' "$WORK/status-detached.log")" 1

if "$AW" status --exit-code "$WORK/status.workspace" >/dev/null; then
	fail "status --exit-code fails for an absent member"
else
	pass "status --exit-code fails for an absent member"
fi
git clone -q "$WORK/origins/missing.git" "$WORK/status.workspace/missing"
git -C "$WORK/status.workspace/missing" config --local user.name "Template User"
git -C "$WORK/status.workspace/missing" config --local user.email template@example.com
git -C "$WORK/status.workspace/missing" update-ref -d refs/remotes/origin/main
if "$AW" status "$WORK/status.workspace" >"$WORK/status-pruned.log" \
	2>"$WORK/status-pruned.err"; then
	pass "status tolerates a pruned tracked ref"
else
	fail "status tolerates a pruned tracked ref"
fi
assert "status reports a pruned tracked ref as no upstream" \
	"$(grep -c '^missing[[:space:]]*no upstream$' "$WORK/status-pruned.log")" 1
if "$AW" status --exit-code "$WORK/status.workspace" >/dev/null; then
	pass "status --exit-code ignores dirty working trees"
else
	fail "status --exit-code ignores dirty working trees"
fi

cat >>"$WORK/status.workspace/workspace.toml" <<EOF

[[repo]]
path = "blocked/child"
url = "$WORK/origins/alpha.git"
EOF
echo obstruction >"$WORK/status.workspace/blocked"
if "$AW" status "$WORK/status.workspace" >"$WORK/status-blocked.out" \
	2>"$WORK/status-blocked.err"; then
	fail "status fails when repository metadata cannot be inspected"
else
	pass "status fails when repository metadata cannot be inspected"
fi
assert "status reports the repository inspection failure" \
	"$(grep -c 'inspecting repository metadata' "$WORK/status-blocked.err")" 1
}

# --- status drift ------------------------------------------------------------
case_status_drift() {
"$AW" init --template "$WORK/seed-template" \
	"$WORK/status-drift.workspace" --name status-drift >/dev/null 2>&1
cat >>"$WORK/status-drift.workspace/workspace.toml" <<EOF

[[repo]]
path = "alpha"
url = "$WORK/origins/alpha.git"

[[repo]]
path = "./relative"
url = "$WORK/origins/beta.git"
EOF
"$AW" bootstrap "$WORK/status-drift.workspace" >/dev/null 2>&1
git -C "$WORK/status-drift.workspace" config --local core.hooksPath .custom-hooks
git init -q --initial-branch=main "$WORK/status-drift.workspace/unlisted"
mkdir -p "$WORK/status-drift.workspace/vendor"
git init -q --initial-branch=main "$WORK/status-drift.workspace/vendor/beta"
ln -s "$WORK/status-drift.workspace/absent-target" \
	"$WORK/status-drift.workspace/dangling"

if "$AW" status "$WORK/status-drift.workspace" >"$WORK/status-unlisted.log"; then
	pass "status ignores a dangling immediate-child symlink"
else
	fail "status ignores a dangling immediate-child symlink"
fi
assert "status reports an unlisted immediate-child checkout" \
	"$(grep -c '^unlisted[[:space:]]*present, not declared$' \
		"$WORK/status-unlisted.log")" 1
assert "status excludes declared members from unlisted checkouts" \
	"$(sed -n '/^unlisted checkouts$/,$p' "$WORK/status-unlisted.log" |
		grep -c '^alpha[[:space:]]' || true)" 0
assert "status normalises a declared checkout path" \
	"$(sed -n '/^unlisted checkouts$/,$p' "$WORK/status-unlisted.log" |
		grep -c '^relative[[:space:]]' || true)" 0
assert "status ignores a nested unlisted checkout" \
	"$(grep -c '^vendor/beta[[:space:]]' "$WORK/status-unlisted.log" || true)" 0
assert "status preserves a user-set workspace hook path" \
	"$(grep -c 'core\.hooksPath' "$WORK/status-unlisted.log" || true)" 0
if "$AW" status --exit-code "$WORK/status-drift.workspace" >/dev/null; then
	pass "status --exit-code ignores unlisted checkouts"
else
	fail "status --exit-code ignores unlisted checkouts"
fi

git -C "$WORK/status-drift.workspace/alpha" \
	config --local user.email drifted@example.com
"$AW" status "$WORK/status-drift.workspace" >"$WORK/status-drift-human.log"
"$AW" status --json "$WORK/status-drift.workspace" >"$WORK/status-drift.json"
assert "status reports exactly one drifted config key" \
	"$(awk '/^configuration drift$/ { section = 1; next }
		/^unlisted checkouts$/ { section = 0 }
		section && NF { count++ }
		END { print count + 0 }' "$WORK/status-drift-human.log")" 1
assert "status names the drifted repository and key" \
	"$(grep -c '^alpha[[:space:]]*user\.email$' \
		"$WORK/status-drift-human.log")" 1
assert "status does not treat the custom hook path as drift" \
	"$(grep -c 'core\.hooksPath' "$WORK/status-drift-human.log" || true)" 0
assert "status JSON includes the configuration-drift section" \
	"$(grep -c '"name": "configuration_drift"' "$WORK/status-drift.json")" 1
assert "status JSON marks configuration drift as a finding" \
	"$(grep -A2 '"name": "configuration_drift"' "$WORK/status-drift.json" |
		grep -c '"finding": true')" 1
assert "status JSON names the drifted config key" \
	"$(grep -c '"user.email"' "$WORK/status-drift.json")" 1
assert "status JSON associates the drifted repository and key" \
	"$(awk '
		/"name": "configuration_drift"/ { section = 1; next }
		section && /"name":/ { section = 0 }
		section && /"repository": "alpha"/ { repository = 1 }
		section && /"user.email"/ { key = 1 }
		END { print (repository && key) + 0 }' "$WORK/status-drift.json")" 1
assert "status JSON includes the unlisted-checkouts section" \
	"$(grep -c '"name": "unlisted_checkouts"' "$WORK/status-drift.json")" 1
assert "status JSON does not classify an unlisted checkout as a finding" \
	"$(grep -A2 '"name": "unlisted_checkouts"' "$WORK/status-drift.json" |
		grep -c '"finding": false')" 1
assert "status JSON names the unlisted checkout path" \
	"$(sed -n '/\"name\": \"unlisted_checkouts\"/,$p' \
		"$WORK/status-drift.json" | grep -c '"path": "unlisted"')" 1
if "$AW" status --exit-code "$WORK/status-drift.workspace" >/dev/null; then
	fail "status --exit-code fails for config drift"
else
	pass "status --exit-code fails for config drift"
fi

git -C "$WORK/status-drift.workspace/alpha" \
	config --local user.email template@example.com
git -C "$WORK/status-drift.workspace/alpha" \
	config --local remote.origin.url "$WORK/origins/beta.git"
"$AW" status "$WORK/status-drift.workspace" >"$WORK/status-origin-drift.log"
assert "status reports member origin drift" \
	"$(grep -c '^alpha[[:space:]]*remote\.origin\.url$' \
		"$WORK/status-origin-drift.log")" 1
"$AW" bootstrap "$WORK/status-drift.workspace" >/dev/null 2>&1
"$AW" status "$WORK/status-drift.workspace" >"$WORK/status-origin-converged.log"
assert "bootstrap converges member origin drift" \
	"$(grep -c '^alpha[[:space:]]*remote\.origin\.url$' \
		"$WORK/status-origin-converged.log" || true)" 0
git -C "$WORK/status-drift.workspace" config --local --unset core.hooksPath
"$AW" status "$WORK/status-drift.workspace" >"$WORK/status-hooks-drift.log"
assert "status reports an unset managed workspace hook" \
	"$(grep -c '^workspace[[:space:]]*core\.hooksPath$' \
		"$WORK/status-hooks-drift.log")" 1
git -C "$WORK/status-drift.workspace" config --local core.hooksPath .custom-hooks

git -C "$WORK/status-drift.workspace" \
	config --local user.email 'template@example.com '
"$AW" status "$WORK/status-drift.workspace" >"$WORK/status-whitespace-drift.log"
assert "status reports semantic identity whitespace as drift" \
	"$(grep -c '^workspace[[:space:]]*user\.email$' \
		"$WORK/status-whitespace-drift.log")" 1
"$AW" bootstrap "$WORK/status-drift.workspace" >/dev/null 2>&1
"$AW" status "$WORK/status-drift.workspace" \
	>"$WORK/status-whitespace-converged.log"
assert "bootstrap converges semantic identity whitespace" \
	"$(grep -c '^workspace[[:space:]]*user\.email$' \
		"$WORK/status-whitespace-converged.log" || true)" 0

cat >>"$WORK/status-drift.workspace/workspace.toml" <<EOF

[[repo]]
path = "missing"
url = "$WORK/origins/beta.git"
EOF
"$AW" status "$WORK/status-drift.workspace" >"$WORK/status-drift-absent.log"
assert "status keeps listed-but-absent classification in presence" \
	"$(grep -c '^missing[[:space:]]*declared, not present$' \
		"$WORK/status-drift-absent.log")" 1
assert "status does not duplicate listed-but-absent drift in new sections" \
	"$(sed -n '/^configuration drift$/,$p' "$WORK/status-drift-absent.log" |
		grep -c '^missing[[:space:]]' || true)" 0
}

# --- sync --------------------------------------------------------------------
case_sync() {
prepare_demo_workspace
"$AW" bootstrap "$WORK/demo.workspace" >/dev/null 2>&1
cat >>"$WORK/demo.workspace/workspace.toml" <<EOF

[[repo]]
path = "missing"
url = "$WORK/origins/alpha.git"
EOF

# Give gamma an isolated origin and advance only its remote branch. A successful
# sync must fetch this commit without moving the checkout's HEAD.
git clone -q --bare "$WORK/origins/gamma.git" "$WORK/gamma-sync.git"
git -C "$WORK/demo.workspace/gamma" remote set-url origin "$WORK/gamma-sync.git"
git clone -q "$WORK/gamma-sync.git" "$WORK/gamma-upstream"
echo "gamma remote update" >>"$WORK/gamma-upstream/file.txt"
git -C "$WORK/gamma-upstream" add file.txt
git -C "$WORK/gamma-upstream" -c user.name=t -c user.email=t@t \
	commit -qm "remote update"
git -C "$WORK/gamma-upstream" push -q origin HEAD:main
GAMMA_REMOTE_HEAD="$(git -C "$WORK/gamma-upstream" rev-parse HEAD)"

# Beta is deliberately dirty and gains a new opted-in skill. Sync may link the
# skill at the workspace layer, but must not alter beta's own status or HEAD.
echo "dirty local" >>"$WORK/demo.workspace/beta/file.txt"
seed_skill "$WORK/demo.workspace/beta" .claude/skills/sync-added \
	"Skill added before sync."

# Alpha's failure must be reported without stopping later member fetches.
git -C "$WORK/demo.workspace/alpha" remote set-url origin \
	"$WORK/unreachable.git"

for name in alpha beta gamma skills; do
	git -C "$WORK/demo.workspace/$name" rev-parse HEAD \
		>"$WORK/$name.head.before"
	git -C "$WORK/demo.workspace/$name" status --porcelain \
		>"$WORK/$name.status.before"
done

REAL_GIT="$(command -v git)"
mkdir -p "$WORK/fake-bin"
cat >"$WORK/fake-bin/git" <<EOF
#!/bin/sh
printf '%s\n' "\$*" >>"\$GIT_LOG"
exec "$REAL_GIT" "\$@"
EOF
chmod +x "$WORK/fake-bin/git"

if GIT_LOG="$WORK/sync-git.log" PATH="$WORK/fake-bin:$PATH" \
	"$AW" sync "$WORK/demo.workspace" >"$WORK/sync.log" \
	2>"$WORK/sync.err"; then
	fail "sync exits non-zero when a member fetch fails"
else
	pass "sync exits non-zero when a member fetch fails"
fi

assert "sync reports the failed member" \
	"$(grep -c '^repo[[:space:]]*alpha[[:space:]]*FAILED' "$WORK/sync.log")" 1
for name in beta gamma skills; do
	assert "sync reports $name fetched" \
		"$(grep -c "^repo[[:space:]]*$name[[:space:]]*fetched" \
			"$WORK/sync.log")" 1
done
assert "sync continues fetching after a failure" \
	"$(grep -c '^-C .*/skills fetch$' "$WORK/sync-git.log")" 1
assert "sync skips an absent member" \
	"$(grep -c '^-C .*/missing fetch$' "$WORK/sync-git.log" || true)" 0
assert "sync does not fetch the workspace repository" \
	"$(grep -c "^-C $WORK/demo.workspace fetch$" "$WORK/sync-git.log" || true)" 0
assert "sync fetches the new remote commit" \
	"$(git -C "$WORK/demo.workspace/gamma" rev-parse refs/remotes/origin/main)" \
	"$GAMMA_REMOTE_HEAD"
assert "sync reports fetch failures on stdout" \
	"$(wc -c <"$WORK/sync.err" | tr -d ' ')" 0

for name in alpha beta gamma skills; do
	git -C "$WORK/demo.workspace/$name" rev-parse HEAD \
		>"$WORK/$name.head.after"
	git -C "$WORK/demo.workspace/$name" status --porcelain \
		>"$WORK/$name.status.after"
	assert "sync leaves $name HEAD unchanged" \
		"$(
			cmp -s "$WORK/$name.head.before" "$WORK/$name.head.after" &&
				echo same || echo changed
		)" same
	assert "sync leaves $name porcelain unchanged" \
		"$(
			cmp -s "$WORK/$name.status.before" "$WORK/$name.status.after" &&
				echo same || echo changed
		)" same
done

for dir in .claude/skills .agents/skills; do
	assert "sync links a newly appeared skill in $dir" \
		"$(grep -c 'Skill added before sync' \
			"$WORK/demo.workspace/$dir/sync-added/SKILL.md")" 1
done
assert "sync reports each changed harness link" \
	"$(grep -c 'skills.*1 link(s) changed' "$WORK/sync.log")" 2
assert "sync reports shadowing during re-link" \
	"$(grep -c '^shadowed[[:space:]]*release' "$WORK/sync.log")" 1

git -C "$WORK/demo.workspace/alpha" remote set-url origin \
	"$WORK/origins/alpha.git"
if "$AW" sync "$WORK/demo.workspace" >"$WORK/sync-clean.log" 2>&1; then
	pass "sync exits zero when every present member fetches"
else
	fail "sync exits zero when every present member fetches"
fi
}

set +e
run_case init case_init
run_case bootstrap case_bootstrap
run_case containment case_containment
run_case adopt case_adopt
run_case doctor case_doctor
run_case status-fixtures case_status_fixtures
run_case status case_status
run_case status-drift case_status_drift
run_case sync case_sync

if [ "$FAILED" -eq 0 ]; then
	echo
	echo "e2e: all assertions passed"
else
	echo
	echo "e2e: FAILURES"
fi
exit "$FAILED"
