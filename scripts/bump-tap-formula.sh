#!/usr/bin/env bash
# bump-tap-formula — point the Homebrew formula at the release just published
#
# Invoked by the publish job in .github/workflows/release.yml, never by hand.
# Rewrites the tag in every download URL and every checksum beside it, then
# commits through the GitHub API so the commit carries GitHub's signature. The
# tap's default branch requires signed commits, and a commit pushed over git
# would be unsigned and rejected. See docs/release-process.md.
#
# Reads VERSION (the tag, e.g. v0.1.1), GH_TOKEN (a token for the tap), and a
# checksums file naming every archive in the release.

set -euo pipefail

VERSION="${VERSION:-}"
SUMS="${SUMS:-dist/SHA256SUMS}"
TAP_REPO="${TAP_REPO:-aw-tools/homebrew-tap}"
FORMULA="${FORMULA:-Formula/aw-cli.rb}"
# Empty means the tap's default branch. Set only to rehearse against a scratch
# branch, which a release never does.
BRANCH="${BRANCH:-}"

if [ -z "$VERSION" ]; then
  echo "VERSION is empty; expected a tag such as v0.1.1." >&2
  exit 2
fi

if [ ! -s "$SUMS" ]; then
  echo "No checksums at ${SUMS}." >&2
  exit 2
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

ref=""
if [ -n "$BRANCH" ]; then
  ref="?ref=${BRANCH}"
fi

gh api "repos/${TAP_REPO}/contents/${FORMULA}${ref}" --jq '.content' | base64 -d > "$work/before.rb"
blob="$(gh api "repos/${TAP_REPO}/contents/${FORMULA}${ref}" --jq '.sha')"

# One pass: a url line yields the archive name and takes the new tag, and the
# sha256 line that follows takes that archive's checksum. Pairing by position
# rather than by pattern means an arm cannot take another arm's checksum.
awk -v version="$VERSION" '
    NR == FNR { sum[$2] = $1; next }

    /url "https:\/\/github\.com\// {
        if (match($0, /\/releases\/download\/[^\/]+\//) == 0) {
            print "Unrecognised download URL: " $0 > "/dev/stderr"
            exit 1
        }
        name = $0
        sub(/^.*\/releases\/download\/[^\/]+\//, "", name)
        sub(/".*$/, "", name)
        if (!(name in sum)) {
            print "No checksum published for " name > "/dev/stderr"
            exit 1
        }
        pending = name
        sub(/\/releases\/download\/[^\/]+\//, "/releases/download/" version "/")
        print
        next
    }

    /^[[:space:]]*sha256 "/ {
        if (pending == "") {
            print "A sha256 line with no download URL before it: " $0 > "/dev/stderr"
            exit 1
        }
        sub(/"[0-9a-f]*"/, "\"" sum[pending] "\"")
        pending = ""
        print
        next
    }

    { print }
' "$SUMS" "$work/before.rb" > "$work/after.rb"

if cmp -s "$work/before.rb" "$work/after.rb"; then
  echo "The formula already points at ${VERSION}; nothing to bump."
  exit 0
fi

# Belt and braces: every arm must now carry this tag and a checksum from this
# release. An arm left behind is internally consistent and installs the old
# version silently, so it is checked rather than trusted.
stale_tag="$(grep -c "releases/download/" "$work/after.rb" || true)"
fresh_tag="$(grep -c "releases/download/${VERSION}/" "$work/after.rb" || true)"
if [ "$stale_tag" != "$fresh_tag" ]; then
  echo "Only ${fresh_tag} of ${stale_tag} download URLs carry ${VERSION}." >&2
  exit 1
fi

while read -r line; do
  digest="${line#*\"}"
  digest="${digest%\"*}"
  if ! grep -q "^${digest}[[:space:]]" "$SUMS"; then
    echo "Checksum ${digest} is not in ${SUMS}." >&2
    exit 1
  fi
done < <(grep '^[[:space:]]*sha256 "' "$work/after.rb")

put=(-f "message=chore: aw-cli ${VERSION}" -f "content=$(base64 < "$work/after.rb" | tr -d '\n')" -f "sha=$blob")
if [ -n "$BRANCH" ]; then
  put+=(-f "branch=$BRANCH")
fi

gh api -X PUT "repos/${TAP_REPO}/contents/${FORMULA}" "${put[@]}" \
  --jq '"Bumped the formula to '"$VERSION"' in \(.commit.sha)"'
