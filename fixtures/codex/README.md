# Codex fixtures

Both files were captured on a real machine by `scripts/probe/codex.mjs` and scrubbed
(emails, account/user ids, paths, instructions and any long string are replaced by `[redacted]`).

| File | Origin |
|---|---|
| `rollout.jsonl` | `session_meta`, last `turn_context` and last `token_count` events of a real rollout (decompressed from `.jsonl.zst`). |
| `wham-usage.json` | Real response of `GET https://chatgpt.com/backend-api/wham/usage` (Plus plan). |
