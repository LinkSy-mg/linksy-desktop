#!/usr/bin/env bash
# Put a release on Flathub: build the submission from the release tag, and (with --pr) open
# the pull request that asks Flathub to list it. See docs/flathub.md for what to do first.
#
# Flathub's copy of the manifest is the one in this repo, plus the tag's exact commit (a
# manifest can't name the commit that contains it, so it's added here). Nothing leaves this
# computer without --pr.
#
# Usage: scripts/flathub-submit.sh [vX.Y.Z] [--pr]     (default: the version in Cargo.toml)
# Needs: git, gh (logged in, for --pr)
set -euo pipefail
cd "$(dirname "$0")/.."

APP=mg.diary.linksy.Linksy
TAG=v$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
PR=
for a in "$@"; do case $a in --pr) PR=1 ;; v*) TAG=$a ;; *) echo "unknown argument: $a"; exit 2 ;; esac; done

git rev-parse -q --verify "refs/tags/$TAG" >/dev/null || { echo "no tag $TAG here: git fetch --tags"; exit 1; }
COMMIT=$(git rev-parse "$TAG^{commit}")
OUT=target/flathub/$TAG
rm -rf "$OUT"; mkdir -p "$OUT"

# The manifest, cargo sources and arches exactly as the tag has them, with the commit pinned.
git show "$TAG:packaging/flatpak/$APP.yml" \
  | sed "s/^\(        tag: $TAG\)\$/\1\n        commit: $COMMIT/" > "$OUT/$APP.yml"
git show "$TAG:packaging/flatpak/cargo-sources.json" > "$OUT/cargo-sources.json"
git show "$TAG:packaging/flatpak/flathub.json" > "$OUT/flathub.json" 2>/dev/null \
  || cp packaging/flatpak/flathub.json "$OUT/flathub.json"
grep -q "^        commit: $COMMIT\$" "$OUT/$APP.yml" \
  || { echo "the manifest at $TAG doesn't name $TAG as its tag"; exit 1; }
echo "Submission for $TAG ($COMMIT) is in $OUT"
[ -n "$PR" ] || { echo "Add --pr to open the Flathub pull request."; exit 0; }

# Flathub wants the pull request against its new-pr branch, from a branch of your fork.
gh repo fork flathub/flathub --clone=false >/dev/null 2>&1 || true
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
gh repo clone flathub/flathub "$WORK" -- --depth 1 --branch new-pr -q
git -C "$WORK" checkout -q -b "add-$APP"
cp "$OUT"/* "$WORK/"
git -C "$WORK" add -A
git -C "$WORK" commit -qm "Add $APP"
USER_LOGIN=$(gh api user -q .login)
git -C "$WORK" push -q "https://github.com/$USER_LOGIN/flathub.git" "add-$APP"
gh pr create -R flathub/flathub --base new-pr --head "$USER_LOGIN:add-$APP" \
  --title "Add $APP" --body-file docs/flathub-pr-body.md
