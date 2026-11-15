# Antigravity fixtures

Captured on a real machine by `scripts/probe/antigravity.mjs` from the local language server
(Antigravity, Google AI Pro plan) and scrubbed: names, emails, ids, URLs and long strings are
`[redacted]`. `supportedMimeTypes` / `allowedTiers` were dropped from `GetUserStatus.json` for size.

| File | Method |
|---|---|
| `RetrieveUserQuotaSummary.json` | Quota pools (`groups[].buckets[]` with `remainingFraction`, `resetTime`, `window`). |
| `GetUserStatus.json` | Plan info and per-model `quotaInfo`. |

`GetCommandModelConfigs` answered HTTP 501 (`unimplemented`) on this version, so there is no capture.
