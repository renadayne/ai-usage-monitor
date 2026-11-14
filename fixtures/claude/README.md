# Claude fixtures

| File | Origin |
|---|---|
| `session.jsonl` | Captured on a real machine by `scripts/probe/claude.mjs`, rebuilt from a field whitelist (no conversation content, session id zeroed). |
| `usage.json` | **Synthetic.** Built from the publicly documented shape of `GET /api/oauth/usage`; the dev machine has no `.credentials.json`, so the endpoint could not be captured. |
| `statusline.json` | **Synthetic.** Shape of the JSON Claude Code passes to a statusline script (`rate_limits`). |
| `credentials.json` | **Synthetic.** Shape of `.credentials.json` with placeholder tokens. |
