// Spike for the GitHub Copilot provider. Read-only.
//
//   node scripts/probe/copilot.mjs
//
// Looks for a GitHub token (Copilot config files, then `gh auth token`), calls
// `copilot_internal/user` ONCE and stores a redacted response in
// fixtures/copilot/user-captured.json. The token is never printed or written.

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const fixtureDir = path.join(root, "fixtures", "copilot");

function tokenFromConfigFiles() {
  const dirs = [
    process.env.LOCALAPPDATA && path.join(process.env.LOCALAPPDATA, "github-copilot"),
    path.join(os.homedir(), ".config", "github-copilot"),
  ].filter(Boolean);
  for (const dir of dirs) {
    for (const name of ["apps.json", "hosts.json"]) {
      const file = path.join(dir, name);
      if (!fs.existsSync(file)) continue;
      try {
        const entries = JSON.parse(fs.readFileSync(file, "utf8"));
        for (const [host, entry] of Object.entries(entries)) {
          if (host.startsWith("github.com") && entry?.oauth_token) {
            console.log(`token: found in ${name}`);
            return entry.oauth_token;
          }
        }
      } catch {
        console.log(`token: ${name} is not valid JSON`);
      }
    }
  }
  return null;
}

function tokenFromGh() {
  try {
    const token = execFileSync("gh", ["auth", "token"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
      windowsHide: true,
    }).trim();
    if (token) console.log("token: found via `gh auth token`");
    return token || null;
  } catch {
    return null;
  }
}

const REDACTED_KEYS = /login|email|tracking|token|organization|endpoint|url/i;

function scrub(value, key = "") {
  if (value === null || typeof value === "number" || typeof value === "boolean") return value;
  if (typeof value === "string") return REDACTED_KEYS.test(key) ? "[redacted]" : value;
  if (Array.isArray(value)) return REDACTED_KEYS.test(key) ? [] : value.map((v) => scrub(v, key));
  return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, scrub(v, k)]));
}

const token = tokenFromConfigFiles() ?? tokenFromGh();
if (!token) {
  console.log("token: none found (no Copilot config files, no GitHub CLI login)");
} else {
  const response = await fetch("https://api.github.com/copilot_internal/user", {
    headers: {
      Authorization: `token ${token}`,
      Accept: "application/json",
      "User-Agent": "ai-usage-monitor-probe",
      "X-GitHub-Api-Version": "2025-04-01",
    },
  });
  console.log(`copilot_internal/user: HTTP ${response.status}`);
  const text = await response.text();
  if (response.ok) {
    const body = JSON.parse(text);
    console.log(`top-level keys: ${Object.keys(body).join(", ")}`);
    console.log(`quota_snapshots keys: ${Object.keys(body.quota_snapshots ?? {}).join(", ")}`);
    fs.mkdirSync(fixtureDir, { recursive: true });
    fs.writeFileSync(
      path.join(fixtureDir, "user-captured.json"),
      JSON.stringify(scrub(body), null, 2) + "\n",
    );
    console.log("wrote fixtures/copilot/user-captured.json");
  } else {
    console.log(`body length = ${text.length}`);
  }
}
