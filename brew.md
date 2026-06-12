# Distributing anki-tui via a Homebrew tap

`anki-tui` is distributed through the personal tap at
[`professionalgriefer/homebrew-tap`](https://github.com/professionalgriefer/homebrew-tap).
The formula installs a **prebuilt binary** that you build locally and attach to a
GitHub Release. This is macOS arm64 only (the machine the binary is built on) —
it is meant for personal use, not general distribution.

## Releasing a new version

### 1. Tag the release

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

### 2. Build the binary

```sh
cargo build --release
```

The binary lands at `target/release/anki-tui`.

### 3. Package it

```sh
tar -czf anki-tui-aarch64-apple-darwin.tar.gz -C target/release anki-tui
shasum -a 256 anki-tui-aarch64-apple-darwin.tar.gz
```

### 4. Create the GitHub Release and upload the tarball

```sh
gh release create v0.1.0 anki-tui-aarch64-apple-darwin.tar.gz \
  --title v0.1.0 --notes "..."
```

The download URL is then:

```
https://github.com/professionalgriefer/anki-tui/releases/download/v0.1.0/anki-tui-aarch64-apple-darwin.tar.gz
```

### 5. Update the formula in the tap

Edit `Formula/anki-tui.rb` in the tap repo:

```ruby
class AnkiTui < Formula
  desc "Keyboard-driven terminal reviewer for Anki (via AnkiConnect)"
  homepage "https://github.com/professionalgriefer/anki-tui"
  url "https://github.com/professionalgriefer/anki-tui/releases/download/v0.1.0/anki-tui-aarch64-apple-darwin.tar.gz"
  sha256 "<sha256 from step 3>"
  version "0.1.0"
  license "MIT"

  depends_on :macos
  depends_on arch: :arm64

  def install
    bin.install "anki-tui"
  end

  test do
    assert_path_exists bin/"anki-tui"
  end
end
```

For each new release, bump the `url` + `version` to the new tag and replace the
`sha256`. That's the entire release cadence.

> **License note:** `Cargo.toml` currently has no `license` field and there's no
> `LICENSE` file. Either add one or correct/remove the `license` line, otherwise
> `brew audit` will warn.

### 6. Commit and push the tap, then install

```sh
brew install professionalgriefer/tap/anki-tui
# or:
brew tap professionalgriefer/tap
brew install anki-tui
```

## Testing the formula locally before pushing

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
