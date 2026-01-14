Release a new version of recall.

Bump type: $ARGUMENTS (patch if not specified)

## Steps

1. Read current version from `Cargo.toml`, compute next version based on bump type (patch/minor/major)

2. Check `git log` since last tag to summarize changes for the commit message

3. Delete any pre-release tags for the new version (e.g., vX.Y.Z-rc1) locally and on remote, if they exist

4. Bump version in `Cargo.toml`

5. Update flake.lock (if flake.nix exists):
   ```bash
   nix flake update
   ```

6. Commit and tag:
   ```bash
   git add -A && git commit -m "vX.Y.Z: <summary of changes>"
   git tag -a vX.Y.Z -m "vX.Y.Z: <summary of changes>"
   git push && git push --tags
   ```

7. Watch GitHub Actions build:
   ```bash
   gh run watch
   ```

8. Download release assets and compute SHA256:
   ```bash
   rm -rf /tmp/release && mkdir -p /tmp/release
   gh release download vX.Y.Z -R zippoxer/recall --pattern "*.tar.gz" -D /tmp/release
   cd /tmp/release && shasum -a 256 *.tar.gz
   ```

9. Verify homebrew-tap was updated correctly by CI:
   ```bash
   # Fetch the formula and verify hashes match
   gh api repos/zippoxer/homebrew-tap/contents/Formula/recall.rb --jq '.content' | base64 -d
   ```
   Compare the SHA256 hashes in the formula with the hashes from step 8:
   - `recall-macos-intel.tar.gz` hash should match `on_intel` block
   - `recall-macos-arm64.tar.gz` hash should match `on_arm` block
   - `recall-linux-x86_64.tar.gz` hash should match `on_linux` block

   If hashes don't match, fix manually in ~/code/homebrew-tap (clone if not present).

10. Verify Homebrew:
   ```bash
   brew update && brew upgrade zippoxer/tap/recall

   # Test with tmux
   tmux new-session -d -s test -x 120 -y 40
   tmux send-keys -t test 'recall test query' Enter
   sleep 2 && tmux capture-pane -t test -p
   tmux kill-session -t test
   ```

11. Verify WinGet (automated via GitHub Actions):
    - Check for PR at https://github.com/microsoft/winget-pkgs/pulls?q=is:pr+zippoxer.recall
    - The `publish-winget` job automatically submits a PR to winget-pkgs for non-prerelease versions
    - PRs are typically merged within 24-48 hours by the winget-pkgs maintainers
