# Distributing anki-tui via a Homebrew tap

`anki-tui` is distributed through the personal tap at
[`professionalgriefer/homebrew-tap`](https://github.com/professionalgriefer/homebrew-tap).
The formula installs a **prebuilt binary** produced by the release workflow and
attached to a GitHub Release. This is macOS arm64 only — it is meant for
personal use, not general distribution.

## One-time setup

Add a fine-grained personal access token named `HOMEBREW_TAP_TOKEN` to the
`anki-tui` repository's GitHub Actions secrets. Give the token **Contents:
Read and write** access to `professionalgriefer/homebrew-tap`.

The release workflow uses this token because the repository-scoped
`GITHUB_TOKEN` for `anki-tui` cannot push to the separate tap repository.

## Releasing a new version

Update the version in `Cargo.toml`, commit it, then tag the release:

```sh
cd anki-tui
jj tag set v0.1.0 -r @-   # tag the release commit (match the version in Cargo.toml)
git push origin v0.1.0    # see note below
```

> **Why `git push` for the tag?** `jj` can create the tag (`jj tag set`, exported
> to Git automatically in a colocated repo), but `jj git push` only pushes
> bookmarks — it does not push tags yet. So the tag itself is pushed with `git`.
> Adjust `-r @-` to whichever revision you're releasing (`@-` is the parent of the
> working-copy commit).

Pushing a `v*` tag triggers `.github/workflows/release.yml`, which:

1. Checks that the tag matches the version in `Cargo.toml`.
2. Builds and packages the macOS arm64 binary.
3. Creates a GitHub Release and uploads the tarball.
4. Updates the tap formula's `url`, `sha256`, and `version`.
5. Commits and pushes the formula change to the tap.

> **License note:** `Cargo.toml` currently has no `license` field and there's no
> `LICENSE` file. Either add one or correct/remove the `license` line, otherwise
> `brew audit` will warn.

After the workflow succeeds, install or upgrade the formula:

```sh
brew install professionalgriefer/tap/anki-tui
# or:
brew tap professionalgriefer/tap
brew install anki-tui
# subsequent releases:
brew upgrade anki-tui
```

## Testing the formula locally

```sh
brew install ./Formula/anki-tui.rb
brew audit --new --formula professionalgriefer/tap/anki-tui   # lint
```

## Notes

- **Gatekeeper:** the binary is built locally and unsigned. Homebrew strips the
  download quarantine attribute on install, so this is normally a non-issue. If
  macOS ever complains that the developer can't be verified, that's the cause;
  only real Apple Developer signing/notarization removes it entirely.
- **Single arch:** the formula is pinned to macOS arm64. To also support Intel
  Macs you'd cross-compile (`rustup target add x86_64-apple-darwin`,
  `cargo build --release --target x86_64-apple-darwin`), upload a second tarball,
  and split the `url`/`sha256` into `on_arm`/`on_intel` blocks.
