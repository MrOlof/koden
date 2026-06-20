#!/usr/bin/env node
// fake-usage-endpoint.mjs -- a zero-dependency local stand-in for Claude's
// /api/oauth/usage endpoint, so Koden's usage guard can be tested against a
// known, settable utilization without burning a real quota.
//
// Point Koden at it by launching the dev app with:
//   KODEN_USAGE_ENDPOINT=http://127.0.0.1:8473
// (set the env var before `pnpm tauri dev`, or in the shell that launches it).
//
// Response shape mirrors the real /api/oauth/usage payload:
//   {
//     "five_hour": { "utilization": <0..1>, "resets_at": "<ISO8601>" }
//   }
//
// Utilization is settable two ways (query param wins per-request, so a single
// running server can simulate a ramp without restarts):
//   - CLI flag:     --pct 0.85   (0..1) or --pct 85 (percent; >1 is /100)
//   - query param:  GET /?pct=0.85   or   /api/oauth/usage?pct=85
//
// Reset time defaults to now + --window minutes (default 300 = 5h). Override the
// absolute reset with --resets-at "<ISO8601>" or ?resets_at=<ISO8601>.

import http from "node:http";

const args = process.argv.slice(2);
function opt(name, fallback) {
  const i = args.indexOf(name);
  return i !== -1 && i + 1 < args.length ? args[i + 1] : fallback;
}
function flag(name) {
  return args.includes(name);
}

if (flag("--help") || flag("-h")) {
  process.stdout.write(
    [
      "fake-usage-endpoint.mjs -- local /api/oauth/usage stand-in for Koden's usage guard.",
      "",
      "Usage: node scripts/fake-usage-endpoint.mjs [options]",
      "",
      "Options:",
      "  --port <n>            Listen port (default: 8473).",
      "  --host <addr>         Bind address (default: 127.0.0.1).",
      "  --pct <v>             Default utilization. 0..1, or a percent if >1 (85 -> 0.85).",
      "  --window <min>        Minutes until the 5h window resets (default: 300).",
      '  --resets-at "<ISO>"   Absolute reset time, overrides --window.',
      "  --help, -h           Show this help.",
      "",
      "Per-request overrides (query params win over flags):",
      "  /?pct=0.85            Set utilization for this request.",
      "  /?resets_at=<ISO8601> Set reset time for this request.",
      "",
      "Point Koden at it: KODEN_USAGE_ENDPOINT=http://127.0.0.1:8473",
      "",
    ].join("\n"),
  );
  process.exit(0);
}

const port = Number.parseInt(opt("--port", "8473"), 10) || 8473;
const host = opt("--host", "127.0.0.1");
const windowMin = Number.parseInt(opt("--window", "300"), 10) || 300;
const defaultResetsAt = opt("--resets-at", null);

function normalizePct(raw, fallback) {
  if (raw === null || raw === undefined || raw === "") return fallback;
  const n = Number.parseFloat(raw);
  if (!Number.isFinite(n) || n < 0) return fallback;
  // Treat >1 as a percentage (e.g. 85 -> 0.85); clamp to [0,1].
  const v = n > 1 ? n / 100 : n;
  return Math.min(1, Math.max(0, v));
}

const defaultPct = normalizePct(opt("--pct", null), 0.0);

function resetsAtIso(override) {
  if (override) return override;
  if (defaultResetsAt) return defaultResetsAt;
  return new Date(Date.now() + windowMin * 60_000).toISOString();
}

const server = http.createServer((req, res) => {
  let url;
  try {
    url = new URL(req.url, `http://${req.headers.host || `${host}:${port}`}`);
  } catch {
    url = { searchParams: new URLSearchParams() };
  }

  const utilization = normalizePct(url.searchParams.get("pct"), defaultPct);
  const resets_at = resetsAtIso(url.searchParams.get("resets_at"));

  const body = JSON.stringify({
    five_hour: { utilization, resets_at },
  });

  res.writeHead(200, {
    "content-type": "application/json",
    "access-control-allow-origin": "*",
    "cache-control": "no-store",
  });
  res.end(body);

  // Stderr log only -- never the response body to a shared console.
  process.stderr.write(
    `[usage-endpoint] ${req.method} ${req.url} -> utilization=${utilization} resets_at=${resets_at}\n`,
  );
});

server.on("error", (err) => {
  process.stderr.write(`[usage-endpoint] error: ${err.message}\n`);
  process.exit(1);
});

server.listen(port, host, () => {
  process.stderr.write(
    `[usage-endpoint] listening on http://${host}:${port}  (default utilization=${defaultPct})\n` +
      `[usage-endpoint] point Koden at it: KODEN_USAGE_ENDPOINT=http://${host}:${port}\n`,
  );
});

process.on("SIGINT", () => {
  process.stderr.write("[usage-endpoint] SIGINT -- shutting down.\n");
  server.close(() => process.exit(0));
});
