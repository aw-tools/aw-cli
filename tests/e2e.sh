#!/bin/sh
# End-to-end test for `aw`.
#
# Builds isolated throwaway workspaces against shared local bare repositories.
# Requires `garden` on PATH.
#
# Each case runs in a subshell that reassigns WORK and is called by name, and
# grep patterns quote `aw` output that contains backticks; shellcheck cannot
# see any of that.
# shellcheck disable=SC2016,SC2030,SC2031,SC2329
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
TEMPLATE_SHA="$(git -C "$WORK/seed-template" rev-parse 'fixture-v1^{commit}')"
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

# An agent definition is a single Markdown file, not a SKILL.md-marked
# directory, so it seeds directly at "$1/$2.md".
seed_agent() { # checkout, path (no extension), description
	mkdir -p "$(dirname "$1/$2.md")"
	printf -- '---\nname: %s\ndescription: %s\n---\n' \
		"$(basename "$2")" "$3" >"$1/$2.md"
}

for name in alpha beta gamma skills; do
	git init -q --bare --initial-branch=main "$WORK/origins/$name.git"
	git init -q --initial-branch=main "$WORK/seed-$name"
	echo "$name" >"$WORK/seed-$name/file.txt"
	case "$name" in
	# alpha never opts in, so its skill must stay invisible.
	alpha) seed_skill "$WORK/seed-$name" .claude/skills/alpha-hidden "Never opted in." ;;
	# beta and gamma both expose `deploy`; beta comes first in manifest order, so
	# it wins the name and gamma's copy is shadowed. beta also opts into agents,
	# with a single agent definition to exercise Phase 3b end to end.
	beta)
		seed_skill "$WORK/seed-$name" .claude/skills/deploy "Beta deploy skill."
		seed_agent "$WORK/seed-$name" .claude/agents/reviewer "Beta reviewer agent."
		;;
	gamma) seed_skill "$WORK/seed-$name" .claude/skills/deploy "Gamma deploy skill." ;;
	# A dedicated skills repository keeps its corpus under a single `src` dir.
	skills)
		seed_skill "$WORK/seed-$name" src/release "Shared release skill."
		seed_skill "$WORK/seed-$name" src/pipeline "Shared pipeline skill."
		;;
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
agents = true

[[repo]]
path = "gamma"
url = "$WORK/origins/gamma.git"
skills = true

[[repo]]
path = "skills"
url = "$WORK/origins/skills.git"
skills = { dirs = ["src"] }
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
	# Divergent has never been fetched and now has no clone entry either, so it
	# is the repository whose measurement age cannot be established at all.
	rm -f "$WORK/status.workspace/divergent/.git/logs/HEAD"

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
assert "workspace-local skill shadows the member copy, not linked twice" \
	"$(grep -Ec 'shadowed  release at .* is ignored because workspace is already sourced' \
		"$WORK/boot1.log")" 1
assert "member repo skill keeps its own name" \
	"$(grep -c 'Beta deploy skill' "$WORK/demo.workspace/.claude/skills/deploy/SKILL.md")" 1
assert "first-appearance wins a cross-repo name collision" \
	"$(grep -Ec 'shadowed  deploy at .* is ignored because member repo is already sourced' \
		"$WORK/boot1.log")" 1
assert "the shadowed copy contributes no namespaced link" \
	"$(find "$WORK/demo.workspace/.claude/skills" -mindepth 1 -maxdepth 1 -name '*deploy*' | wc -l | tr -d ' ')" 1
assert "opted-out repo contributes nothing" \
	"$(find "$WORK/demo.workspace/.claude/skills" -mindepth 1 -maxdepth 1 -name '*alpha-hidden*' | wc -l | tr -d ' ')" 0
for dir in .claude/skills .agents/skills; do
	assert "dedicated skills repo links a skill from its src dir in $dir" \
		"$(grep -c 'Shared pipeline skill' \
			"$WORK/demo.workspace/$dir/pipeline/SKILL.md")" 1
done

assert "agent definition links into .claude/agents" \
	"$(grep -c 'Beta reviewer agent' "$WORK/demo.workspace/.claude/agents/reviewer.md")" 1
assert "agent link is relative" \
	"$(readlink "$WORK/demo.workspace/.claude/agents/reviewer.md" | cut -c1-2)" ..
assert "bootstrap reports the linked agent definition" \
	"$(grep -Ec '^agent +reviewer +beta$' "$WORK/boot1.log")" 1

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
	"$(grep -c ' 0 link(s) changed' "$WORK/boot2.log")" 3
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

# A configured source directory that does not exist is a typo the operator must
# see, so doctor reports it as a finding.
sed 's|skills = { dirs = \["src"\] }|skills = { dirs = ["src", "absent"] }|' \
	"$WORK/demo.workspace/workspace.toml" >"$WORK/demo.workspace/workspace.toml.next"
mv "$WORK/demo.workspace/workspace.toml.next" "$WORK/demo.workspace/workspace.toml"
if "$AW" doctor "$WORK/demo.workspace" >"$WORK/doctor-missing-dir.log" 2>&1; then
	fail "doctor fails on a configured skills dir that does not exist"
else
	pass "doctor fails on a configured skills dir that does not exist"
fi
assert "doctor names the missing configured skills directory" \
	"$(grep -Ec '^FAIL  skills dir .*skills configured skills directory absent does not exist' \
		"$WORK/doctor-missing-dir.log")" 1
"$AW" bootstrap "$WORK/demo.workspace" >"$WORK/bootstrap-missing-dir.log" 2>&1
assert "bootstrap warns about the missing configured skills directory" \
	"$(grep -c 'skills configured skills directory absent does not exist' \
		"$WORK/bootstrap-missing-dir.log")" 1
if "$AW" sync "$WORK/demo.workspace" >"$WORK/sync-missing-dir.log" 2>&1; then
	pass "sync exits zero despite a missing configured skills directory"
else
	fail "sync exits zero despite a missing configured skills directory"
fi
assert "sync warns about the missing configured skills directory" \
	"$(grep -c 'skills configured skills directory absent does not exist' \
		"$WORK/sync-missing-dir.log")" 1
sed 's|skills = { dirs = \["src", "absent"\] }|skills = { dirs = ["src"] }|' \
	"$WORK/demo.workspace/workspace.toml" >"$WORK/demo.workspace/workspace.toml.next"
mv "$WORK/demo.workspace/workspace.toml.next" "$WORK/demo.workspace/workspace.toml"

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

case_doctor_pinned_branch() {
git init -q --bare --initial-branch=main "$WORK/develop.git"
git init -q --initial-branch=develop "$WORK/seed-develop"
echo develop >"$WORK/seed-develop/file.txt"
git -C "$WORK/seed-develop" add file.txt
git -C "$WORK/seed-develop" -c user.name=t -c user.email=t@t commit -qm develop
git -C "$WORK/seed-develop" push -q "$WORK/develop.git" HEAD:develop

"$AW" init --template "$WORK/seed-template" \
	"$WORK/pinned.workspace" --name pinned >"$WORK/init-pinned.log" 2>&1
cat >>"$WORK/pinned.workspace/workspace.toml" <<EOF

[[repo]]
path = "member"
url = "$WORK/develop.git"
branch = "develop"
EOF
"$AW" bootstrap "$WORK/pinned.workspace" >"$WORK/bootstrap-pinned.log" 2>&1
if "$AW" doctor "$WORK/pinned.workspace" >"$WORK/doctor-pinned.log" 2>&1; then
	pass "doctor accepts an existing pinned branch when remote HEAD dangles"
else
	fail "doctor accepts an existing pinned branch when remote HEAD dangles"
fi
assert "doctor reports the existing pinned branch reachable" \
	"$(grep -c '^PASS  remote.*member.*reachable' "$WORK/doctor-pinned.log")" 1

sed '/^branch = "develop"$/d' \
	"$WORK/pinned.workspace/workspace.toml" >"$WORK/pinned.workspace/workspace.toml.next"
mv "$WORK/pinned.workspace/workspace.toml.next" \
	"$WORK/pinned.workspace/workspace.toml"
if "$AW" doctor "$WORK/pinned.workspace" >"$WORK/doctor-unpinned.log" 2>&1; then
	fail "doctor keeps probing remote HEAD for an unpinned member"
else
	pass "doctor keeps probing remote HEAD for an unpinned member"
fi
assert "doctor reports dangling remote HEAD for an unpinned member" \
	"$(grep -c '^FAIL  remote.*member' "$WORK/doctor-unpinned.log")" 1

"$AW" init --template "$WORK/seed-template" \
	"$WORK/missing.workspace" --name missing >"$WORK/init-missing.log" 2>&1
cat >>"$WORK/missing.workspace/workspace.toml" <<EOF

[[repo]]
path = "member"
url = "$WORK/origins/alpha.git"
branch = "main"
EOF
"$AW" bootstrap "$WORK/missing.workspace" >"$WORK/bootstrap-missing.log" 2>&1
sed 's/^branch = "main"$/branch = "missing"/' \
	"$WORK/missing.workspace/workspace.toml" >"$WORK/missing.workspace/workspace.toml.next"
mv "$WORK/missing.workspace/workspace.toml.next" \
	"$WORK/missing.workspace/workspace.toml"
if "$AW" doctor "$WORK/missing.workspace" >"$WORK/doctor-missing.log" 2>&1; then
	fail "doctor rejects a missing pinned branch"
else
	pass "doctor rejects a missing pinned branch"
fi
assert "doctor reports the missing pinned branch as a finding" \
	"$(grep -c '^FAIL  remote.*member' "$WORK/doctor-missing.log")" 1
assert "doctor names the missing pinned branch instead of blaming the network" \
	"$(grep -c '^FAIL  remote.*member.*pinned branch missing not found on remote' \
		"$WORK/doctor-missing.log")" 1
}

# --- reusable status fixtures -------------------------------------------------
case_status_fixtures() {
setup_status_fixtures

assert "ahead fixture is ahead" \
	"$(git -C "$WORK/status.workspace/ahead" rev-list --left-right --count \
		'HEAD...@{upstream}' | tr '\t' ' ')" "1 0"
assert "behind fixture is behind" \
	"$(git -C "$WORK/status.workspace/behind" rev-list --left-right --count \
		'HEAD...@{upstream}' | tr '\t' ' ')" "0 1"
assert "divergent fixture is ahead and behind" \
	"$(git -C "$WORK/status.workspace/divergent" rev-list --left-right --count \
		'HEAD...@{upstream}' | tr '\t' ' ')" "1 1"
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
		/^hook health$/ { section = 0 }
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

[[repo]]
path = "workspace-alias"
url = "$WORK/origins/alpha.git"
EOF
ln -s . "$WORK/demo.workspace/workspace-alias"

# Give gamma an isolated origin and advance only its remote branch. A successful
# sync must fetch this commit without moving the checkout's HEAD.
git clone -q --bare "$WORK/origins/gamma.git" "$WORK/gamma-sync.git"
git -C "$WORK/demo.workspace/gamma" remote set-url origin "$WORK/gamma-sync.git"
git clone -q "$WORK/gamma-sync.git" "$WORK/gamma-upstream"
echo "gamma remote update" >>"$WORK/gamma-upstream/file.txt"
git -C "$WORK/gamma-upstream" add file.txt
git -C "$WORK/gamma-upstream" -c user.name=t -c user.email=t@t \
	commit -qm "remote update"
git -C "$WORK/gamma-upstream" tag sync-marker
git -C "$WORK/gamma-upstream" push -q origin HEAD:main refs/tags/sync-marker
GAMMA_REMOTE_HEAD="$(git -C "$WORK/gamma-upstream" rev-parse HEAD)"

# A branch-selected alternate remote and an origin refspec targeting a local
# branch must not affect which remote or namespace sync refreshes.
git -C "$WORK/demo.workspace/gamma" remote add alternate \
	"$WORK/origins/beta.git"
git -C "$WORK/demo.workspace/gamma" config branch.main.remote alternate
git -C "$WORK/demo.workspace/gamma" config --add remote.origin.fetch \
	'+refs/heads/main:refs/heads/sync-clobber'

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
assert "sync rejects a member resolving to the workspace repository" \
	"$(grep -c '^repo[[:space:]]*workspace-alias[[:space:]]*FAILED' \
		"$WORK/sync.log")" 1
for name in beta gamma skills; do
	assert "sync reports $name fetched" \
		"$(grep -c "^repo[[:space:]]*${name}[[:space:]]*fetched" \
			"$WORK/sync.log")" 1
done
assert "sync continues fetching after a failure" \
	"$(grep -c '^-C .*/skills fetch ' "$WORK/sync-git.log")" 1
assert "sync skips an absent member" \
	"$(grep -c '^-C .*/missing fetch ' "$WORK/sync-git.log" || true)" 0
assert "sync skips a workspace repository with no origin" \
	"$(grep -c "^-C $WORK/demo.workspace fetch " "$WORK/sync-git.log" || true)" 0
assert "sync reports the origin-less workspace as skipped" \
	"$(grep -c '^layer[[:space:]]*workspace[[:space:]]*no origin, skipped' \
		"$WORK/sync.log")" 1
assert "sync does not fetch a workspace-repository alias" \
	"$(grep -c "^-C $WORK/demo.workspace/workspace-alias fetch " \
		"$WORK/sync-git.log" || true)" 0
assert "sync fetches the new remote commit" \
	"$(git -C "$WORK/demo.workspace/gamma" rev-parse refs/remotes/origin/main)" \
	"$GAMMA_REMOTE_HEAD"
if git -C "$WORK/demo.workspace/gamma" show-ref --verify --quiet \
	refs/tags/sync-marker; then
	fail "sync does not auto-follow tags"
else
	pass "sync does not auto-follow tags"
fi
if git -C "$WORK/demo.workspace/gamma" show-ref --verify --quiet \
	refs/heads/sync-clobber; then
	fail "sync ignores configured refspecs targeting local branches"
else
	pass "sync ignores configured refspecs targeting local branches"
fi
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
	"$(grep -Ec '^shadowed  release at .* is ignored because workspace is already sourced' \
		"$WORK/sync.log")" 1

rm "$WORK/demo.workspace/workspace-alias"
ln -s gamma "$WORK/demo.workspace/workspace-alias"
git -C "$WORK/demo.workspace/alpha" remote set-url origin \
	"$WORK/origins/alpha.git"
if "$AW" sync "$WORK/demo.workspace" >"$WORK/sync-clean.log" 2>&1; then
	pass "sync exits zero when every present member fetches"
else
	fail "sync exits zero when every present member fetches"
fi
# The verify tally counts members only; the workspace layer reports its own row
# and never inflates these numbers.
assert "sync counts only members in the verify tally" \
	"$(grep -c '^verify    5 repo(s), 1 skipped, 0 failed,' "$WORK/sync-clean.log")" 1

# The workspace layer is a repository too, and status reports its ahead/behind
# beside the members'. Give it an origin, advance that origin, and require sync
# to refresh the remote-tracking ref without moving the checkout.
git -C "$WORK/demo.workspace" add README.md
git -C "$WORK/demo.workspace" -c user.name=t -c user.email=t@t \
	commit -qm "workspace layer"
git init -q --bare "$WORK/workspace-origin.git"
git -C "$WORK/workspace-origin.git" symbolic-ref HEAD refs/heads/trunk
git -C "$WORK/demo.workspace" remote add origin "$WORK/workspace-origin.git"
git -C "$WORK/demo.workspace" push -q origin HEAD:trunk
git clone -q "$WORK/workspace-origin.git" "$WORK/workspace-upstream"
echo "layer update" >>"$WORK/workspace-upstream/README.md"
git -C "$WORK/workspace-upstream" add README.md
git -C "$WORK/workspace-upstream" -c user.name=t -c user.email=t@t \
	commit -qm "layer update"
git -C "$WORK/workspace-upstream" push -q origin HEAD:trunk
WORKSPACE_REMOTE_HEAD="$(git -C "$WORK/workspace-upstream" rev-parse HEAD)"
WORKSPACE_HEAD_BEFORE="$(git -C "$WORK/demo.workspace" rev-parse HEAD)"
git -C "$WORK/demo.workspace" status --porcelain \
	>"$WORK/workspace.status.before"

if "$AW" sync "$WORK/demo.workspace" >"$WORK/sync-workspace.log" 2>&1; then
	pass "sync exits zero when the workspace layer fetches"
else
	fail "sync exits zero when the workspace layer fetches"
fi
assert "sync reports the workspace layer fetched" \
	"$(grep -c '^layer[[:space:]]*workspace[[:space:]]*fetched' \
		"$WORK/sync-workspace.log")" 1
assert "sync fetches the workspace layer's new remote commit" \
	"$(git -C "$WORK/demo.workspace" rev-parse refs/remotes/origin/trunk)" \
	"$WORKSPACE_REMOTE_HEAD"
assert "sync leaves the workspace layer HEAD unchanged" \
	"$(git -C "$WORK/demo.workspace" rev-parse HEAD)" "$WORKSPACE_HEAD_BEFORE"
git -C "$WORK/demo.workspace" status --porcelain >"$WORK/workspace.status.after"
assert "sync leaves the workspace layer porcelain unchanged" \
	"$(diff -q "$WORK/workspace.status.before" "$WORK/workspace.status.after" \
		>/dev/null && echo same || echo differs)" same

# The reported age must follow the last fetch, not the last time a ref moved.
# Discard the remote-tracking reflog and sync again: the second fetch brings
# nothing new, and status must still call the measurement fresh rather than
# falling through to the unknown-age hint.
git -C "$WORK/demo.workspace" branch --set-upstream-to=origin/trunk >/dev/null
rm -f "$WORK/demo.workspace/.git/logs/refs/remotes/origin/trunk"
if "$AW" sync "$WORK/demo.workspace" >"$WORK/sync-workspace-again.log" 2>&1; then
	pass "sync exits zero on a second workspace-layer fetch"
else
	fail "sync exits zero on a second workspace-layer fetch"
fi
if "$AW" status "$WORK/demo.workspace" >"$WORK/status-workspace-age.log" 2>&1; then
	pass "status exits zero after the second workspace-layer fetch"
else
	fail "status exits zero after the second workspace-layer fetch"
fi
assert "status ages the workspace layer from the fetch, not the last ref move" \
	"$(grep -Ec '^workspace[[:space:]]+ahead [0-9]+, behind [0-9]+ \(fetched [0-9]+s ago\)$' \
		"$WORK/status-workspace-age.log")" 1

# A workspace layer that cannot be fetched is reported and fails the command,
# the same way an unreachable member does.
git -C "$WORK/demo.workspace" remote set-url origin "$WORK/unreachable.git"
if "$AW" sync "$WORK/demo.workspace" >"$WORK/sync-workspace-fail.log" 2>&1; then
	fail "sync exits non-zero when the workspace layer cannot be fetched"
else
	pass "sync exits non-zero when the workspace layer cannot be fetched"
fi
assert "sync reports the workspace layer failure" \
	"$(grep -c '^layer[[:space:]]*workspace[[:space:]]*FAILED' \
		"$WORK/sync-workspace-fail.log")" 1

# A root that carries the manifest but is not a repository is skipped, never a
# hard error: status already models that root as absent.
git -C "$WORK/demo.workspace" remote set-url origin "$WORK/workspace-origin.git"
mv "$WORK/demo.workspace/.git" "$WORK/workspace-dotgit"
if "$AW" sync "$WORK/demo.workspace" >"$WORK/sync-workspace-absent.log" 2>&1; then
	pass "sync exits zero when the workspace root is not a repository"
else
	fail "sync exits zero when the workspace root is not a repository"
fi
assert "sync reports a non-repository workspace root as skipped" \
	"$(grep -c '^layer[[:space:]]*workspace[[:space:]]*not a repository, skipped' \
		"$WORK/sync-workspace-absent.log")" 1
mv "$WORK/workspace-dotgit" "$WORK/demo.workspace/.git"

# --- concurrency ---
# `--concurrency 1` is the documented way back to the sequential behaviour, so
# its stdout must stay in manifest order: the layer first, then every member in
# the order workspace.toml lists them.
if "$AW" sync --concurrency 1 "$WORK/demo.workspace" \
	>"$WORK/sync-serial.log" 2>"$WORK/sync-serial.err"; then
	pass "sync exits zero with --concurrency 1"
else
	fail "sync exits zero with --concurrency 1"
fi
assert "sync --concurrency 1 reports in manifest order" \
	"$(grep -E '^(layer|repo) ' "$WORK/sync-serial.log" | awk '{print $2}' |
		tr '\n' ' ')" \
	"workspace alpha beta gamma skills missing workspace-alias "

# Redirected output carries no progress indicator in either stream: the
# indicator is drawn only when stderr is a terminal.
assert "sync --concurrency 1 writes nothing to stderr" \
	"$(wc -c <"$WORK/sync-serial.err" | tr -d ' ')" 0
assert "sync --concurrency 1 leaves no escape sequences on stdout" \
	"$(grep -c "$(printf '\033')" "$WORK/sync-serial.log" || true)" 0

# A concurrent run reports the same rows, in some order, and writes no progress
# artefacts when redirected.
if "$AW" sync --concurrency 8 "$WORK/demo.workspace" \
	>"$WORK/sync-parallel.log" 2>"$WORK/sync-parallel.err"; then
	pass "sync exits zero with --concurrency 8"
else
	fail "sync exits zero with --concurrency 8"
fi
assert "sync --concurrency 8 reports every repository exactly once" \
	"$(grep -E '^(layer|repo) ' "$WORK/sync-parallel.log" | awk '{print $2}' |
		sort | tr '\n' ' ')" \
	"alpha beta gamma missing skills workspace workspace-alias "
assert "sync --concurrency 8 writes nothing to stderr" \
	"$(wc -c <"$WORK/sync-parallel.err" | tr -d ' ')" 0
assert "sync --concurrency 8 leaves no escape sequences on stdout" \
	"$(grep -c "$(printf '\033')" "$WORK/sync-parallel.log" || true)" 0
assert "sync --concurrency 8 keeps the verify tally" \
	"$(grep -c '^verify    5 repo(s), 1 skipped, 0 failed,' \
		"$WORK/sync-parallel.log")" 1

# Zero is a plausible thing to type and must fetch sequentially rather than
# leaving the pool with no workers and silently fetching nothing.
if "$AW" sync --concurrency 0 "$WORK/demo.workspace" \
	>"$WORK/sync-zero.log" 2>&1; then
	pass "sync exits zero with --concurrency 0"
else
	fail "sync exits zero with --concurrency 0"
fi
assert "sync --concurrency 0 still fetches every present member" \
	"$(grep -c '^repo[[:space:]].*fetched$' "$WORK/sync-zero.log")" 5

# The manifest carries the workspace-wide default.
cat >>"$WORK/demo.workspace/workspace.toml" <<EOF

[sync]
concurrency = 2
EOF
if "$AW" sync "$WORK/demo.workspace" >"$WORK/sync-configured.log" 2>&1; then
	pass "sync accepts sync.concurrency from the manifest"
else
	fail "sync accepts sync.concurrency from the manifest"
fi
assert "sync with a configured concurrency fetches every present member" \
	"$(grep -c '^repo[[:space:]].*fetched$' "$WORK/sync-configured.log")" 5

# A typo in the table is rejected outright rather than silently ignored.
sed -i.bak 's/^concurrency = 2$/concurrancy = 2/' \
	"$WORK/demo.workspace/workspace.toml"
if "$AW" sync "$WORK/demo.workspace" >"$WORK/sync-typo.log" 2>&1; then
	fail "sync rejects an unknown key in the sync table"
else
	pass "sync rejects an unknown key in the sync table"
fi
assert "sync names the unknown sync key and the one it meant" \
	"$(grep -c 'unknown field `concurrancy`, expected `concurrency`' \
		"$WORK/sync-typo.log")" 1
mv "$WORK/demo.workspace/workspace.toml.bak" \
	"$WORK/demo.workspace/workspace.toml"
}

# --- fast-forward -------------------------------------------------------------
# One member per outcome, each with its own origin so each can sit where the
# case needs it. Members listed in the order the report is read back.
FF_PRESENT="advance current detached feature nohead noupstream elsewhere dirty picking diverged blocked later tagged"

ff_member() { # name
	git clone -q --bare "$WORK/ff-seed" "$WORK/ff-origins/$1.git"
	git clone -q "$WORK/ff-origins/$1.git" "$WORK/ff.workspace/$1"
}

ff_advance_origin() { # name, file
	git clone -q "$WORK/ff-origins/$1.git" "$WORK/ff-upstream-$1"
	echo "$1 upstream" >"$WORK/ff-upstream-$1/$2"
	git -C "$WORK/ff-upstream-$1" add "$2"
	git -C "$WORK/ff-upstream-$1" -c user.name=t -c user.email=t@t \
		commit -qm "upstream $1"
	git -C "$WORK/ff-upstream-$1" push -q origin HEAD:main
}

ff_snapshot() { # suffix
	for name in $FF_PRESENT; do
		git -C "$WORK/ff.workspace/$name" rev-parse HEAD >"$WORK/$name.head.$1"
		git -C "$WORK/ff.workspace/$name" status --porcelain >"$WORK/$name.status.$1"
	done
}

ff_same() { # name, file stem, suffix a, suffix b
	cmp -s "$WORK/$1.$2.$3" "$WORK/$1.$2.$4" && echo same || echo changed
}

case_fast_forward() {
"$AW" init --template "$WORK/seed-template" \
	"$WORK/ff.workspace" --name ff >"$WORK/ff-init.log" 2>&1
git init -q --initial-branch=main "$WORK/ff-seed"
echo base >"$WORK/ff-seed/file.txt"
git -C "$WORK/ff-seed" add file.txt
git -C "$WORK/ff-seed" -c user.name=t -c user.email=t@t commit -qm base
mkdir -p "$WORK/ff-origins"
for name in $FF_PRESENT absent; do
	printf '\n[[repo]]\npath = "%s"\nurl = "%s"\n' \
		"$name" "$WORK/ff-origins/$name.git" >>"$WORK/ff.workspace/workspace.toml"
done
for name in $FF_PRESENT; do
	ff_member "$name"
done
for name in advance dirty picking diverged later tagged; do
	ff_advance_origin "$name" file.txt
done
# The upstream adds a file the checkout already holds untracked, so git refuses
# the move.
ff_advance_origin blocked new.txt
echo 'local untracked' >"$WORK/ff.workspace/blocked/new.txt"

M="$WORK/ff.workspace"
# A member pushed rather than cloned has no origin/HEAD; the verb creates it.
git -C "$M/advance" remote set-head origin -d
git -C "$M/detached" switch -q --detach
git -C "$M/feature" switch -qc feature
# Origin names a default branch that does not exist, so none can be learnt.
git -C "$WORK/ff-origins/nohead.git" symbolic-ref HEAD refs/heads/gone
git -C "$M/nohead" remote set-head origin -d
git -C "$M/noupstream" branch -q --unset-upstream
# Main tracks another branch on origin, which is ahead, so a move would pull that
# branch into main.
git clone -q "$WORK/ff-origins/elsewhere.git" "$WORK/ff-upstream-elsewhere"
echo 'release upstream' >"$WORK/ff-upstream-elsewhere/file.txt"
git -C "$WORK/ff-upstream-elsewhere" -c user.name=t -c user.email=t@t \
	commit -qam 'upstream release'
git -C "$WORK/ff-upstream-elsewhere" push -q origin HEAD:release
git -C "$M/elsewhere" config branch.main.merge refs/heads/release
# A tag and a local branch whose short names collide with main and origin/main
# must not hide that the member is on its default branch.
git -C "$M/tagged" tag main
git -C "$M/tagged" update-ref refs/heads/origin/main HEAD
echo 'local edit' >>"$M/dirty/file.txt"
git -C "$M/picking" rev-parse HEAD >"$M/picking/.git/CHERRY_PICK_HEAD"
echo 'local commit' >>"$M/diverged/file.txt"
git -C "$M/diverged" -c user.name=t -c user.email=t@t commit -qam local

ff_snapshot before
if "$AW" ff --dry-run --concurrency 1 "$M" >"$WORK/ff-dry.log" 2>&1; then
	pass "fast-forward --dry-run exits zero when nothing fails"
else
	fail "fast-forward --dry-run exits zero when nothing fails"
fi
ff_snapshot dry
for name in $FF_PRESENT; do
	assert "fast-forward --dry-run leaves $name HEAD unchanged" \
		"$(ff_same "$name" head before dry)" same
	assert "fast-forward --dry-run leaves $name porcelain unchanged" \
		"$(ff_same "$name" status before dry)" same
done
assert "fast-forward --dry-run reports what would advance" \
	"$(grep -Ec '^repo +advance +would advance [0-9a-f]+\.\.[0-9a-f]+ \(1 commit\(s\)\)$' \
		"$WORK/ff-dry.log")" 1
assert "fast-forward --dry-run creates a missing origin/HEAD" \
	"$(git -C "$M/advance" symbolic-ref refs/remotes/origin/HEAD)" \
	refs/remotes/origin/main
assert "fast-forward --dry-run tallies what would advance" \
	"$(grep -c '^verify    4 would advance, 1 up to date, 9 skipped, 0 failed$' \
		"$WORK/ff-dry.log")" 1

if "$AW" fast-forward --concurrency 1 "$M" >"$WORK/ff.log" 2>&1; then
	fail "fast-forward exits non-zero when a move fails"
else
	pass "fast-forward exits non-zero when a move fails"
fi
ff_snapshot after
for name in advance later tagged; do
	assert "fast-forward moves $name to its upstream" \
		"$(git -C "$M/$name" rev-parse HEAD)" \
		"$(git -C "$WORK/ff-origins/$name.git" rev-parse main)"
	assert "fast-forward reports $name advanced" \
		"$(grep -Ec "^repo +$name +advanced [0-9a-f]+\.\.[0-9a-f]+ \(1 commit\(s\)\)$" \
			"$WORK/ff.log")" 1
done
assert "fast-forward reports an up-to-date member" \
	"$(grep -Ec '^repo +current +up to date$' "$WORK/ff.log")" 1
assert "fast-forward reports the refused move with git's reason" \
	"$(grep -Ec '^repo +blocked +FAILED — .*untracked working tree files would be overwritten' \
		"$WORK/ff.log")" 1
assert "fast-forward keeps the untracked file in the way" \
	"$(cat "$M/blocked/new.txt")" 'local untracked'
for pair in \
	'detached:detached HEAD' \
	'feature:on feature, not the default branch main' \
	'nohead:default branch unknown: error: Cannot determine remote HEAD' \
	'noupstream:no upstream branch' \
	'elsewhere:tracks origin/release, not origin/main' \
	'dirty:tracked changes' \
	'picking:cherry-pick in progress' \
	'diverged:ahead 1, behind 1' \
	'absent:not present, skipped'; do
	name="${pair%%:*}"
	why="${pair#*:}"
	case "$name" in absent) expected="$why" ;; *) expected="skipped — $why" ;; esac
	assert "fast-forward skips $name: $why" \
		"$(grep -c "^repo      $name *$expected\$" "$WORK/ff.log")" 1
done
for name in current detached feature nohead noupstream elsewhere dirty picking diverged blocked; do
	assert "fast-forward leaves $name HEAD unchanged" \
		"$(ff_same "$name" head before after)" same
done
assert "fast-forward tallies the run" \
	"$(grep -c '^verify    3 advanced, 1 up to date, 9 skipped, 1 failed$' \
		"$WORK/ff.log")" 1
assert "fast-forward reports no workspace-layer row" \
	"$(grep -c '^layer' "$WORK/ff.log" || true)" 0
}

# --- read-only invariant ------------------------------------------------------
case_readonly_invariant() {
prepare_demo_workspace
"$AW" bootstrap "$WORK/demo.workspace" >/dev/null 2>&1

# Skills stays clean. Beta has tracked and untracked changes. Gamma's isolated
# origin advances after the clone. Alpha's origin cannot be reached.
echo "dirty local" >>"$WORK/demo.workspace/beta/file.txt"
echo "untracked local" >"$WORK/demo.workspace/beta/untracked.txt"

git clone -q --bare "$WORK/origins/gamma.git" "$WORK/gamma-readonly.git"
git -C "$WORK/demo.workspace/gamma" remote set-url origin \
	"$WORK/gamma-readonly.git"
git clone -q "$WORK/gamma-readonly.git" "$WORK/gamma-upstream"
echo "gamma remote update" >>"$WORK/gamma-upstream/file.txt"
git -C "$WORK/gamma-upstream" add file.txt
git -C "$WORK/gamma-upstream" -c user.name=t -c user.email=t@t \
	commit -qm "remote update"
git -C "$WORK/gamma-upstream" push -q origin HEAD:main

git -C "$WORK/demo.workspace/alpha" remote set-url origin \
	"$WORK/unreachable.git"

assert "read-only invariant fixture includes a clean member" \
	"$(git -C "$WORK/demo.workspace/skills" status --porcelain | wc -l | tr -d ' ')" 0
assert "read-only invariant fixture includes tracked and untracked changes" \
	"$(git -C "$WORK/demo.workspace/beta" status --porcelain |
		grep -Ec '^ M file\.txt$|^\?\? untracked\.txt$')" 2
if [ "$(git -C "$WORK/gamma-readonly.git" rev-parse main)" != \
	"$(git -C "$WORK/demo.workspace/gamma" rev-parse HEAD)" ]; then
	pass "read-only invariant fixture includes a remote-ahead member"
else
	fail "read-only invariant fixture includes a remote-ahead member"
fi

for name in alpha beta gamma skills; do
	git -C "$WORK/demo.workspace/$name" rev-parse HEAD \
		>"$WORK/$name.head.before"
	git -C "$WORK/demo.workspace/$name" status --porcelain \
		>"$WORK/$name.status.before"
done

if "$AW" sync "$WORK/demo.workspace" >"$WORK/sync.log" 2>&1; then
	fail "read-only invariant fixture reports an unreachable member"
else
	pass "read-only invariant fixture reports an unreachable member"
fi
assert "read-only invariant fixture names the unreachable member" \
	"$(grep -c '^repo[[:space:]]*alpha[[:space:]]*FAILED' "$WORK/sync.log")" 1

for name in alpha beta gamma skills; do
	git -C "$WORK/demo.workspace/$name" rev-parse HEAD \
		>"$WORK/$name.head.after"
	git -C "$WORK/demo.workspace/$name" status --porcelain \
		>"$WORK/$name.status.after"
	assert "read-only invariant: sync leaves $name HEAD unchanged" \
		"$(
			cmp -s "$WORK/$name.head.before" "$WORK/$name.head.after" &&
				echo same || echo changed
		)" same
	assert "read-only invariant: sync leaves $name porcelain unchanged" \
		"$(
			cmp -s "$WORK/$name.status.before" "$WORK/$name.status.after" &&
				echo same || echo changed
		)" same
done
}

# --- status skill links -------------------------------------------------------
case_status_skills() {
prepare_demo_workspace
"$AW" bootstrap "$WORK/demo.workspace" >/dev/null 2>&1
mkdir -p "$WORK/demo.workspace/.agents/skills/mine"
cat >"$WORK/demo.workspace/.agents/skills/mine/SKILL.md" <<'EOF'
---
name: mine
description: User-owned skill.
---
EOF

"$AW" status --exit-code "$WORK/demo.workspace" \
	>"$WORK/status-skills-healthy.log"
"$AW" status --json "$WORK/demo.workspace" \
	>"$WORK/status-skills-healthy.json"
sed -n '/"name": "skill_links"/,$p' "$WORK/status-skills-healthy.json" \
	>"$WORK/status-skills-healthy-section.json"
assert "status reports a resolved workspace skill" \
	"$(grep -c '^release[[:space:]]*resolved from workspace$' \
		"$WORK/status-skills-healthy.log")" 1
assert "status reports a resolved member skill" \
	"$(grep -c '^deploy[[:space:]]*resolved from member repo$' \
		"$WORK/status-skills-healthy.log")" 1
assert "status reports both origins for a shadowed skill" \
	"$(grep -c '^release[[:space:]]*shadowed: member repo by workspace$' \
		"$WORK/status-skills-healthy.log")" 1
assert "status reports a user-owned directory as untouched" \
	"$(grep -c '^mine[[:space:]]*user-owned in Codex at .agents/skills/mine; untouched$' \
		"$WORK/status-skills-healthy.log")" 1
assert "status leaves a user-owned directory in place" \
	"$([ -d "$WORK/demo.workspace/.agents/skills/mine" ] && echo yes)" yes
assert "status JSON includes the skill-links section" \
	"$(grep -c '"name": "skill_links"' "$WORK/status-skills-healthy.json")" 1
assert "status JSON marks healthy skill-link state as not a finding" \
	"$(grep -A2 '"name": "skill_links"' "$WORK/status-skills-healthy.json" |
		grep -c '"finding": false')" 1
assert "status JSON reports the resolved workspace skill" \
	"$(grep -B2 -A2 '"skill": "release"' \
		"$WORK/status-skills-healthy-section.json" |
		grep -c '"origin": "workspace"')" 1
assert "status JSON reports the resolved member skill" \
	"$(grep -B2 -A2 '"skill": "deploy"' \
		"$WORK/status-skills-healthy-section.json" |
		grep -c '"origin": "member repo"')" 1
assert "status JSON reports the complete shadowed skill" \
	"$(grep -B2 -A2 '"skill": "release"' \
		"$WORK/status-skills-healthy-section.json" |
		grep -Ec '"loser_origin": "member repo"|"winner_origin": "workspace"')" 2
assert "status JSON reports the complete user-owned directory" \
	"$(grep -B2 -A2 '"skill": "mine"' \
		"$WORK/status-skills-healthy-section.json" |
		grep -Ec '"harness": "Codex"|"path": ".agents/skills/mine"')" 2

ln -s ../../.skills/gone \
	"$WORK/demo.workspace/.claude/skills/broken"
ln -s ../../.skills/gone \
	"$WORK/demo.workspace/.agents/skills/broken"
"$AW" status "$WORK/demo.workspace" >"$WORK/status-skills-dangling.log"
"$AW" status --json "$WORK/demo.workspace" \
	>"$WORK/status-skills-dangling.json"
sed -n '/"name": "skill_links"/,$p' "$WORK/status-skills-dangling.json" \
	>"$WORK/status-skills-dangling-section.json"
assert "status reports the dangling Codex skill link" \
	"$(grep -c '^broken[[:space:]]*dangling link at .agents/skills/broken$' \
		"$WORK/status-skills-dangling.log")" 1
assert "status reports the dangling Claude Code skill link" \
	"$(grep -c '^broken[[:space:]]*dangling link at .claude/skills/broken$' \
		"$WORK/status-skills-dangling.log")" 1
assert "status JSON marks a dangling skill link as a finding" \
	"$(grep -A2 '"name": "skill_links"' "$WORK/status-skills-dangling.json" |
		grep -c '"finding": true')" 1
assert "status JSON reports the complete dangling Codex skill link" \
	"$(grep -B2 -A2 '"skill": "broken"' \
		"$WORK/status-skills-dangling-section.json" |
		grep -c '"path": ".agents/skills/broken"')" 1
assert "status JSON reports the complete dangling Claude Code skill link" \
	"$(grep -B2 -A2 '"skill": "broken"' \
		"$WORK/status-skills-dangling-section.json" |
		grep -c '"path": ".claude/skills/broken"')" 1
if "$AW" status --exit-code "$WORK/demo.workspace" >/dev/null; then
	fail "status --exit-code fails for a dangling skill link"
else
	pass "status --exit-code fails for a dangling skill link"
fi
assert "status still leaves the user-owned directory in place" \
	"$([ -d "$WORK/demo.workspace/.agents/skills/mine" ] && echo yes)" yes
}

set +e
run_case init case_init
run_case bootstrap case_bootstrap
run_case containment case_containment
run_case adopt case_adopt
run_case doctor case_doctor
run_case doctor-pinned-branch case_doctor_pinned_branch
run_case status-fixtures case_status_fixtures
run_case status case_status
run_case status-drift case_status_drift
run_case sync case_sync
run_case fast-forward case_fast_forward
run_case read-only-invariant case_readonly_invariant
run_case status-skills case_status_skills

if [ "$FAILED" -eq 0 ]; then
	echo
	echo "e2e: all assertions passed"
else
	echo
	echo "e2e: FAILURES"
fi
exit "$FAILED"
