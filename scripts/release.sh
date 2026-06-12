#!/usr/bin/env bash

set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

tag="${1:-}"
tap_repo="${HOMEBREW_TAP_REPO:-"$repo_root/../homebrew-tap"}"
target="aarch64-apple-darwin"
archive="anki-tui-$target.tar.gz"
formula="$tap_repo/Formula/anki-tui.rb"

if [[ ! "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: scripts/release.sh vX.Y.Z" >&2
  exit 1
fi

for command in cargo gh git jq ruby rustup shasum tar; do
  command -v "$command" >/dev/null || {
    echo "missing required command: $command" >&2
    exit 1
  }
done

if [[ ! -d "$tap_repo/.git" || ! -f "$formula" ]]; then
  echo "Homebrew tap not found at $tap_repo" >&2
  echo "Set HOMEBREW_TAP_REPO to its checkout path." >&2
  exit 1
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "anki-tui has uncommitted tracked changes" >&2
  exit 1
fi

if [[ -n "$(git -C "$tap_repo" status --porcelain)" ]]; then
  echo "Homebrew tap has uncommitted changes: $tap_repo" >&2
  exit 1
fi

if ! git rev-parse --verify --quiet "refs/tags/$tag" >/dev/null; then
  echo "tag does not exist locally: $tag" >&2
  exit 1
fi

cargo_version="$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "anki-tui") | .version')"
if [[ "${tag#v}" != "$cargo_version" ]]; then
  echo "tag version ${tag#v} does not match Cargo.toml version $cargo_version" >&2
  exit 1
fi

if [[ "$(git rev-list -n 1 "$tag")" != "$(git rev-parse HEAD)" ]]; then
  echo "tag $tag does not point at HEAD" >&2
  exit 1
fi

rustup target add "$target"
cargo build --release --locked --target "$target"
tar -czf "$archive" -C "target/$target/release" anki-tui
sha256="$(shasum -a 256 "$archive" | awk '{print $1}')"

git push origin "$tag"

if gh release view "$tag" >/dev/null 2>&1; then
  gh release upload "$tag" "$archive" --clobber
else
  gh release create "$tag" "$archive" --generate-notes --verify-tag
fi

ruby scripts/update-homebrew-formula.rb "$formula" "$tag" "$sha256"

git -C "$tap_repo" add Formula/anki-tui.rb
if git -C "$tap_repo" diff --cached --quiet; then
  echo "Homebrew formula is already up to date"
else
  git -C "$tap_repo" commit -m "anki-tui $tag"
  git -C "$tap_repo" push
fi

echo "Released $tag and updated the Homebrew tap"
