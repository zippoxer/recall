# recall

Search and resume your Claude Code and Codex CLI conversations.

![screenshot](screenshot-dark.png)

## Install

**Homebrew** (macOS/Linux):
```bash
brew install zippoxer/tap/recall
```

**Cargo**:
```bash
cargo install --git https://github.com/zippoxer/recall
```

**Binary**: Download from [Releases](https://github.com/zippoxer/recall/releases)

## Use

```bash
recall
```

**That's it.** Start typing to search. Enter to jump back in.

| Key | Action |
|-----|--------|
| `↑↓` | Navigate results |
| `Pg↑/↓` | Scroll preview |
| `Enter` | Resume conversation |
| `Tab` | Copy session ID |
| `/` | Toggle scope (folder/everywhere) |
| `Esc` | Quit |

## Customize

recall's resume commands can be configured with environment variables.

For example, to resume conversations in YOLO mode, add this to your `.bashrc` or `.zshrc`:
```bash
export RECALL_CLAUDE_CMD="claude --dangerously-skip-permissions --resume {id}"
export RECALL_CODEX_CMD="codex --dangerously-bypass-approvals-and-sandbox resume {id}"
```

---

![light mode](screenshot-light.png)

---

Made with ❤️ by [zippoxer](https://github.com/zippoxer) and Claude.
