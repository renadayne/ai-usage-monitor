// Spike for the Claude provider. Read-only.
//
//   node scripts/probe/claude.mjs
//
// - Looks for the Claude Code config dir and credentials.
// - Calls the OAuth usage endpoint ONCE (if a non-expired token exists) and
//   stores the response in fixtures/claude/usage.json.
// - Stores a redacted tail of the newest session log in fixtures/claude/session.jsonl.
//
// Nothing secret is printed or written: tokens stay in memory, fixtures are
// rebuilt from a whitelist of fields.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const fixtureDir = path.join(root, "fixtures", "claude");
fs.mkdirSync(fixtureDir, { recursive: true });

const claudeDir = process.env.CLAUDE_CONFIG_DIR || path.join(os.homedir(), ".claude");
console.log(`config dir exists: ${fs.existsSync(claudeDir)}`);

// --- credentials + usage endpoint -------------------------------------------
const credentialsPath = path.join(claudeDir, ".credentials.json");
if (!fs.existsSync(credentialsPath)) {
  console.log("credentials: .credentials.json not found (usage endpoint not probed)");
} else {
  const credentials = JSON.parse(fs.readFileSync(credentialsPath, "utf8"));
  const oauth = credentials.claudeAiOauth ?? {};
  console.log(`credentials: top-level keys = ${Object.keys(credentials).join(", ")}`);
  console.log(`credentials: oauth keys = ${Object.keys(oauth).join(", ")}`);
  console.log(`credentials: subscriptionType = ${oauth.subscriptionType ?? "?"}`);
  const expired = typeof oauth.expiresAt === "number" && oauth.expiresAt <= Date.now();
  console.log(`credentials: expired = ${expired}`);

  if (oauth.accessToken && !expired) {
    const response = await fetch("https://api.anthropic.com/api/oauth/usage", {
      headers: {
        Authorization: `Bearer ${oauth.accessToken}`,
        "anthropic-beta": "oauth-2025-04-20",
        "User-Agent": "ai-usage-monitor-probe",
      },
    });
    console.log(`usage endpoint: HTTP ${response.status}`);
    console.log(`usage endpoint: retry-after = ${response.headers.get("retry-after")}`);
    const text = await response.text();
    if (response.ok) {
      // The body only holds percentages and reset times; pretty-print as-is.
      fs.writeFileSync(
        path.join(fixtureDir, "usage.json"),
        JSON.stringify(JSON.parse(text), null, 2) + "\n",
      );
      console.log("usage endpoint: wrote fixtures/claude/usage.json");
    } else {
      console.log(`usage endpoint: body length = ${text.length}`);
    }
  }
}

// --- newest session log ------------------------------------------------------
function newestJsonl(dir, depth = 0) {
  let best = null;
  if (depth > 4 || !fs.existsSync(dir)) return best;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    let candidate = null;
    if (entry.isDirectory()) candidate = newestJsonl(full, depth + 1);
    else if (entry.name.endsWith(".jsonl"))
      candidate = { file: full, mtime: fs.statSync(full).mtimeMs };
    if (candidate && (!best || candidate.mtime > best.mtime)) best = candidate;
  }
  return best;
}

function tail(file, bytes) {
  const size = fs.statSync(file).size;
  const start = Math.max(0, size - bytes);
  const fd = fs.openSync(file, "r");
  const buffer = Buffer.alloc(size - start);
  fs.readSync(fd, buffer, 0, buffer.length, start);
  fs.closeSync(fd);
  const lines = buffer.toString("utf8").split("\n");
  if (start > 0) lines.shift(); // first line is probably cut in half
  return lines.filter((line) => line.trim() !== "");
}

function numbersOnly(value) {
  if (!value || typeof value !== "object") return undefined;
  return Object.fromEntries(Object.entries(value).filter(([, v]) => typeof v === "number"));
}

function redact(line) {
  let entry;
  try {
    entry = JSON.parse(line);
  } catch {
    return null;
  }
  const out = { type: entry.type, timestamp: entry.timestamp };
  if (entry.sessionId) out.sessionId = "00000000-0000-0000-0000-000000000000";
  if (entry.version) out.version = entry.version;
  if (entry.message && typeof entry.message === "object") {
    out.message = { role: entry.message.role };
    if (entry.message.model) out.message.model = entry.message.model;
    if (entry.message.usage) out.message.usage = numbersOnly(entry.message.usage);
    out.message.content = "[redacted]";
  }
  return JSON.stringify(out);
}

const newest = newestJsonl(path.join(claudeDir, "projects"));
if (!newest) {
  console.log("session log: none found");
} else {
  const lines = tail(newest.file, 2 * 1024 * 1024);
  const types = {};
  for (const line of lines) {
    try {
      const type = JSON.parse(line).type;
      types[type] = (types[type] ?? 0) + 1;
    } catch {
      types["<unparseable>"] = (types["<unparseable>"] ?? 0) + 1;
    }
  }
  console.log(`session log: ${lines.length} tail lines, types = ${JSON.stringify(types)}`);
  console.log(`session log: longest line = ${Math.max(...lines.map((l) => l.length))} chars`);
  const redacted = lines.map(redact).filter(Boolean).slice(-25);
  fs.writeFileSync(path.join(fixtureDir, "session.jsonl"), redacted.join("\n") + "\n");
  console.log(
    `session log: wrote ${redacted.length} redacted lines to fixtures/claude/session.jsonl`,
  );
}
