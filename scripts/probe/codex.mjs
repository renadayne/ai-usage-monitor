// Spike for the Codex provider. Read-only.
//
//   node scripts/probe/codex.mjs
//
// - Reports the rollout file formats found under ~/.codex/sessions.
// - Extracts redacted `token_count` / `turn_context` samples from a rollout
//   (plain or zstd) into fixtures/codex/rollout.jsonl.
// - Calls the ChatGPT usage endpoint ONCE and stores a redacted response in
//   fixtures/codex/wham-usage.json.
//
// Tokens stay in memory; nothing secret is printed or written.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import readline from "node:readline";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const fixtureDir = path.join(root, "fixtures", "codex");
fs.mkdirSync(fixtureDir, { recursive: true });

const codexDir = process.env.CODEX_HOME || path.join(os.homedir(), ".codex");
console.log(`config dir exists: ${fs.existsSync(codexDir)}`);

// --- rollout files -----------------------------------------------------------
function walk(dir, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(full, out);
    else out.push({ file: full, ...fs.statSync(full) });
  }
  return out;
}

const rollouts = walk(path.join(codexDir, "sessions")).filter((f) =>
  /rollout-.*\.jsonl(\.zst)?$/.test(f.file),
);
const byExt = {};
for (const f of rollouts) {
  const ext = f.file.endsWith(".zst") ? ".jsonl.zst" : ".jsonl";
  byExt[ext] = (byExt[ext] ?? 0) + 1;
}
console.log(`rollouts: ${rollouts.length} files, formats = ${JSON.stringify(byExt)}`);
rollouts.sort((a, b) => b.mtimeMs - a.mtimeMs);
if (rollouts[0]) {
  console.log(
    `rollouts: newest mtime = ${new Date(rollouts[0].mtimeMs).toISOString()}, size = ${rollouts[0].size}`,
  );
}

const REDACTED_KEYS = /email|account|user_id|token|cwd|path|instructions|summary|writable_roots/i;

function scrub(value, key = "") {
  if (value === null || typeof value === "number" || typeof value === "boolean") return value;
  if (typeof value === "string") {
    if (REDACTED_KEYS.test(key)) return "[redacted]";
    return value.length > 80 ? "[redacted]" : value;
  }
  if (Array.isArray(value)) return value.slice(0, 3).map((item) => scrub(item, key));
  return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, scrub(v, k)]));
}

async function sampleRollout(file) {
  let stream = fs.createReadStream(file);
  if (file.endsWith(".zst")) stream = stream.pipe(zlib.createZstdDecompress());
  const lines = readline.createInterface({ input: stream, crlfDelay: Infinity });
  const samples = { token_count: [], turn_context: [], session_meta: [] };
  const types = {};
  let count = 0;
  for await (const line of lines) {
    count++;
    if (line.length > 200_000) continue;
    let entry;
    try {
      entry = JSON.parse(line);
    } catch {
      continue;
    }
    const kind =
      entry.type === "event_msg" ? `event_msg:${entry.payload?.type}` : String(entry.type);
    types[kind] = (types[kind] ?? 0) + 1;
    if (entry.type === "event_msg" && entry.payload?.type === "token_count") {
      samples.token_count.push(entry);
    } else if (entry.type === "turn_context") {
      samples.turn_context.push(entry);
    } else if (entry.type === "session_meta") {
      samples.session_meta.push({ ...entry, payload: { id: "[redacted]" } });
    }
    if (count > 400_000) break;
  }
  return { samples, types, count };
}

// The smallest file is enough for a format sample (the newest can be huge).
const sampleFile = [...rollouts].sort((a, b) => a.size - b.size).find((f) => f.size > 20_000);
if (sampleFile) {
  const { samples, types, count } = await sampleRollout(sampleFile.file);
  console.log(`sample rollout: ${count} lines, kinds = ${JSON.stringify(types)}`);
  const picked = [
    ...samples.session_meta.slice(0, 1),
    ...samples.turn_context.slice(-1),
    ...samples.token_count.filter((e) => e.payload?.rate_limits).slice(-3),
  ].map((entry) => JSON.stringify(scrub(entry)));
  fs.writeFileSync(path.join(fixtureDir, "rollout.jsonl"), picked.join("\n") + "\n");
  console.log(
    `sample rollout: wrote ${picked.length} redacted lines to fixtures/codex/rollout.jsonl`,
  );
}

// --- live usage endpoint -----------------------------------------------------
const authPath = path.join(codexDir, "auth.json");
if (!fs.existsSync(authPath)) {
  console.log("auth: auth.json not found");
} else {
  const auth = JSON.parse(fs.readFileSync(authPath, "utf8"));
  console.log(`auth: keys = ${Object.keys(auth).join(", ")}; auth_mode = ${auth.auth_mode}`);
  console.log(`auth: api key present = ${Boolean(auth.OPENAI_API_KEY)}`);
  const tokens = auth.tokens ?? {};
  if (tokens.access_token) {
    const headers = {
      Authorization: `Bearer ${tokens.access_token}`,
      "User-Agent": "ai-usage-monitor-probe",
      Accept: "application/json",
    };
    if (tokens.account_id) headers["ChatGPT-Account-Id"] = tokens.account_id;
    const response = await fetch("https://chatgpt.com/backend-api/wham/usage", { headers });
    console.log(`wham/usage: HTTP ${response.status}`);
    const text = await response.text();
    if (response.ok) {
      fs.writeFileSync(
        path.join(fixtureDir, "wham-usage.json"),
        JSON.stringify(scrub(JSON.parse(text)), null, 2) + "\n",
      );
      console.log("wham/usage: wrote fixtures/codex/wham-usage.json");
    } else {
      console.log(`wham/usage: body length = ${text.length}`);
    }
  }
}
