#!/usr/bin/env bash
# ---
# description: Publish the crate to crates.io, then tag v<version> and push the tag, after checking version, changelog and tests
# risky: true
# args:
#   version:
#     pattern: '[0-9]+\.[0-9]+\.[0-9]+'
#     help: Version being released, must match Cargo.toml and have a CHANGELOG.md section
# ---
set -euo pipefail
cd "$CANIF_PROJECT"

tag=v$CANIF_VERSION
manifest=$(grep -m1 -E '^version = "' Cargo.toml | sed -E 's/^version = "(.*)"/\1/')
[[ $manifest == "$CANIF_VERSION" ]] || { echo "Cargo.toml has $manifest, not $CANIF_VERSION"; exit 1; }
grep -qxF "## $CANIF_VERSION" CHANGELOG.md || { echo "CHANGELOG.md has no '## $CANIF_VERSION' section"; exit 1; }
[[ -z $(git status --porcelain) ]] || { echo "working tree not clean"; exit 1; }
if git rev-parse --quiet --verify "refs/tags/$tag" >/dev/null; then
  echo "tag $tag already exists"
  exit 1
fi

cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo publish --dry-run --locked

# Publishing cannot be undone, a tag can: publish first, tag once it succeeded.
cargo publish --locked
git tag -a "$tag" -m "$tag"
git push origin "$tag"
