// Spike for the Antigravity provider. Read-only; Antigravity must be running.
//
//   node scripts/probe/antigravity.mjs
//
// Finds the Antigravity language server, reads its CSRF token from the command
// line (kept in memory only), finds the Connect port among the ports the
// process listens on and calls the three quota methods. Redacted responses go
// to fixtures/antigravity/<Method>.json.
//
// TLS verification is disabled for 127.0.0.1 ONLY (self-signed local cert).

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import https from "node:https";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const fixtureDir = path.join(root, "fixtures", "antigravity");
const HOST = "127.0.0.1";
const SERVICE = "exa.language_server_pb.LanguageServerService";
const METHODS = ["RetrieveUserQuotaSummary", "GetUserStatus", "GetCommandModelConfigs"];

function powershell(script) {
  return execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], {
    encoding: "utf8",
    windowsHide: true,
    maxBuffer: 32 * 1024 * 1024,
  });
}

function flag(args, name) {
  // Supports both `--flag VALUE` and `--flag=VALUE`.
  for (let i = 0; i < args.length; i++) {
    if (args[i] === `--${name}`) return args[i + 1];
    if (args[i].startsWith(`--${name}=`)) return args[i].slice(name.length + 3);
  }
  return undefined;
}

function splitCommandLine(raw) {
  const args = [];
  const pattern = /"([^"]*)"|(\S+)/g;
  let match;
  while ((match = pattern.exec(raw))) args.push(match[1] ?? match[2]);
  return args;
}

const raw = powershell(
  "Get-CimInstance Win32_Process | Where-Object { $_.Name -like 'language_server*' } | " +
    "Select-Object ProcessId,Name,ExecutablePath,CommandLine | ConvertTo-Json -Compress",
).trim();
const processes = raw ? [].concat(JSON.parse(raw)) : [];
const servers = processes.filter((p) =>
  `${p.ExecutablePath} ${p.CommandLine}`.toLowerCase().includes("antigravity"),
);
console.log(`language servers: ${processes.length} found, ${servers.length} belong to Antigravity`);
if (servers.length === 0) {
  console.log("Antigravity is not running; nothing to probe.");
  process.exit(0);
}

const REDACTED_KEYS = /email|name|token|key|url|uri|id$|path|avatar|picture|prompt/i;
const KEEP_KEYS = /^(model|label|displayName|modelName|planName|teamsTier|userTier|tierName)$/i;

function scrub(value, key = "") {
  if (value === null || typeof value === "number" || typeof value === "boolean") return value;
  if (typeof value === "string") {
    if (KEEP_KEYS.test(key)) return value;
    if (REDACTED_KEYS.test(key) || value.length > 60) return "[redacted]";
    return value;
  }
  if (Array.isArray(value)) return value.map((item) => scrub(item, key));
  return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, scrub(v, k)]));
}

function post(protocol, port, method, csrf, body) {
  const client = protocol === "https" ? https : http;
  return new Promise((resolve) => {
    const request = client.request(
      {
        host: HOST, // never anything but loopback
        port,
        path: `/${SERVICE}/${method}`,
        method: "POST",
        rejectUnauthorized: false,
        timeout: 4000,
        headers: {
          "Content-Type": "application/json",
          "Connect-Protocol-Version": "1",
          "X-Codeium-Csrf-Token": csrf,
        },
      },
      (response) => {
        let text = "";
        response.on("data", (chunk) => (text += chunk));
        response.on("end", () => resolve({ status: response.statusCode, text }));
      },
    );
    request.on("timeout", () => request.destroy(new Error("timeout")));
    request.on("error", (error) => resolve({ status: 0, text: "", error: error.code ?? "error" }));
    request.end(JSON.stringify(body));
  });
}

for (const server of servers) {
  const args = splitCommandLine(server.CommandLine ?? "");
  const flagNames = args.filter((a) => a.startsWith("--")).map((a) => a.split("=")[0]);
  console.log(`pid ${server.ProcessId} (${server.Name}): flags = ${flagNames.join(" ")}`);
  const csrf = flag(args, "csrf_token");
  console.log(
    `  csrf_token present = ${Boolean(csrf)}, extension_server_port = ${flag(args, "extension_server_port")}, ` +
      `extension_server_csrf_token present = ${Boolean(flag(args, "extension_server_csrf_token"))}`,
  );
  if (!csrf) continue;

  const netstat = execFileSync("netstat.exe", ["-ano", "-p", "TCP"], {
    encoding: "utf8",
    windowsHide: true,
  });
  const ports = [
    ...new Set(
      netstat
        .split("\n")
        .map((line) => line.trim().split(/\s+/))
        .filter((f) => f[0] === "TCP" && f[3] === "LISTENING" && f[4] === String(server.ProcessId))
        .map((f) => Number(f[1].split(":").pop())),
    ),
  ];
  console.log(`  listening ports: ${ports.join(", ")}`);

  const metadata = { ideName: "antigravity", extensionName: "antigravity", locale: "en" };
  let connect = null;
  for (const port of ports) {
    for (const protocol of ["https", "http"]) {
      const probe = await post(protocol, port, "GetUnleashData", csrf, { metadata });
      console.log(`  probe ${protocol}://${HOST}:${port} -> ${probe.status || probe.error}`);
      if (probe.status === 200) {
        connect = { protocol, port };
        break;
      }
    }
    if (connect) break;
  }
  if (!connect) {
    console.log("  no Connect port answered 200");
    continue;
  }

  fs.mkdirSync(fixtureDir, { recursive: true });
  for (const method of METHODS) {
    let result = await post(connect.protocol, connect.port, method, csrf, { metadata });
    if (result.status !== 200) {
      const retry = await post(connect.protocol, connect.port, method, csrf, {});
      if (retry.status === 200) result = retry;
    }
    console.log(`  ${method}: HTTP ${result.status}, ${result.text.length} bytes`);
    if (result.status === 200) {
      try {
        const body = JSON.parse(result.text);
        console.log(`    top-level keys: ${Object.keys(body).join(", ")}`);
        fs.writeFileSync(
          path.join(fixtureDir, `${method}.json`),
          JSON.stringify(scrub(body), null, 2) + "\n",
        );
      } catch {
        console.log("    response is not JSON");
      }
    } else if (result.text) {
      console.log(`    error body: ${result.text.slice(0, 200)}`);
    }
  }
  break;
}
