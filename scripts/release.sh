#!/usr/bin/env bash
set -euo pipefail

BUMP="${1:-}"
if [[ "$BUMP" != "major" && "$BUMP" != "minor" && "$BUMP" != "patch" ]]; then
    echo "Usage: $0 major|minor|patch" >&2
    exit 1
fi

CURRENT=$(awk -F'"' '/^version/ {print $2; exit}' Cargo.toml)
IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT"

case "$BUMP" in
  major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
  minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
  patch) PATCH=$((PATCH + 1)) ;;
esac

NEW_VERSION="${MAJOR}.${MINOR}.${PATCH}"
TAG="v${NEW_VERSION}"

echo "Bumping $CURRENT -> $NEW_VERSION"

sed -i '' "s/^version = \"${CURRENT}\"/version = \"${NEW_VERSION}\"/" Cargo.toml
cargo check --quiet

git add Cargo.toml Cargo.lock
git commit -m "Release ${TAG}"
git tag "${TAG}"
git push origin main "${TAG}"

echo "Waiting for release workflow..."
sleep 5  # give GitHub a moment to register the run
gh run watch --repo "$(gh repo view --json nameWithOwner -q .nameWithOwner)" \
    "$(gh run list --repo "$(gh repo view --json nameWithOwner -q .nameWithOwner)" \
        --workflow release.yml --limit 1 --json databaseId -q '.[0].databaseId')"
