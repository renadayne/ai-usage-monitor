# Copilot fixtures

**All three files are synthetic.** The dev machine has no discoverable Copilot token
(no `apps.json` / `hosts.json`, no GitHub CLI), so `GET https://api.github.com/copilot_internal/user`
could not be captured.

| File | Shape |
|---|---|
| `user-premium.json` | Publicly documented pre-June-2026 shape: `quota_snapshots.premium_interactions` (premium requests). |
| `user-credits.json` | **Guess** at the GitHub AI Credits shape (a non-unlimited `quota_snapshots` entry whose key contains `credit`). Replace with a real capture from `scripts/probe/copilot.mjs` as soon as one is available. |
| `user-free.json` | Documented Copilot Free shape: `limited_user_quotas` + `monthly_quotas`. |
