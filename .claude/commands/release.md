Release a new version of recall.

Bump type: $ARGUMENTS (patch if not specified)

## Steps

1. Read current version from `Cargo.toml`, compute next version based on bump type (patch/minor/major)

2. Check `git log` since last tag to summarize changes for the commit message

3. Delete any pre-release tags for the new version (e.g., vX.Y.Z-rc1) locally and on remote, if they exist

4. Bump version in `Cargo.toml`

5. Commit and tag:
   ```bash
   git add -A && git commit -m "vX.Y.Z: <summary of changes>"
   git tag -a vX.Y.Z -m "vX.Y.Z: <summary of changes>"
   git push && git push --tags
   ```

6. Watch GitHub Actions build:
   ```bash
   gh run watch
   ```

7. Download release assets and compute SHA256:
   ```bash
   rm -rf /tmp/release && mkdir -p /tmp/release
   gh release download vX.Y.Z -R zippoxer/recall --pattern "*.tar.gz" -D /tmp/release
   cd /tmp/release && shasum -a 256 *.tar.gz
   ```

8. Update homebrew-tap (clone to ~/code/homebrew-tap if not present):
   - Update version and SHA256 hashes in `Formula/recall.rb`
   - Commit: "Update recall to vX.Y.Z"
   - Push

9. Verify:
   ```bash
   brew update && brew upgrade zippoxer/tap/recall

   # Test with tmux
   tmux new-session -d -s test -x 120 -y 40
   tmux send-keys -t test 'recall test query' Enter
   sleep 2 && tmux capture-pane -t test -p
   tmux kill-session -t test
   ```
