#!/usr/bin/env node
// fake-claude.mjs -- a zero-dependency Claude Code impersonator for exercising
// Koden's REAL agent + retry detector pipeline without a real CLI, network, or
// rate limit.
//
// Run this INSIDE a Koden terminal tab. Its stdout flows through the real PTY
// -> the per-session byte reader in src-tauri/.../pty/session.rs -> AgentDetector
// (agent_detect.rs) + RetryDetector (retry_detect.rs). Those emit the same
// `koden:agent-signal` / `koden:retry-signal` events the GUI listens for, so
// the AgentDock status, the retry toggle, and the auto-resume bridge all behave
// exactly as they would against a real Claude session.
//
// What it does, in order:
//   1. ARM the session as "claude" so the detector treats subsequent output as
//      an agent run. Two arming modes:
//        - default  : OSC 133;C;claude  (shell-integration command-start marker)
//        - --no-shell: OSC 777;notify;Koden;working (the hook self-arm marker,
//                      for bash/Windows/tmux where no shell preexec fired)
//   2. Print a usage-limit banner in the modern format (middle-dot separator,
//      lowercase am/pm, IANA tz in parens). Default reset is now+2min so the
//      retry bridge schedules a SHORT, observable wait.
//   3. Print the auto-opened "What do you want to do?" menu.
//   4. Read stdin and echo back, byte-for-byte (with hex), whatever Koden
//      injects -- so the tester SEES the resume text / CR / Esc arrive. Runs
//      until Ctrl-C.
//
// IMPORTANT -- detector contract (read before changing the banner text):
//   retry_detect.rs::parse_reset fires when the ANSI-stripped, lowercased output
//   contains a limit phrase AND a derivable time. As of the modern-CLI fix the
//   limit phrases include "you've hit your", "you're out of extra usage", and
//   "now using extra usage" -- so the literal "you've hit your <kind> limit"
//   that the v2.1.x CLI actually prints DOES trigger a retry on its own; it no
//   longer needs the legacy "limit reached" substring. The time clause is one
//   of: "resets <clock>" / "resets at <clock>" / "try again in N <unit>".
//   The DEFAULT banner below therefore uses the EXACT modern phrasing the
//   retry_detect tests assert fire (e.g. session ==
//   "You've hit your session limit \u{00b7} resets <clock> (<tz>)"), so this
//   harness genuinely exercises the fixed detector's modern path rather than the
//   legacy fallback. Use --reset "<text>" to inject any arbitrary reset string
//   (including ones that intentionally do NOT match, to test the negative path).

const ESC = "\x1b";
const ST = `${ESC}\\`; // String Terminator: ESC backslash

const args = process.argv.slice(2);
function flag(name) {
  return args.includes(name);
}
function opt(name, fallback) {
  const i = args.indexOf(name);
  return i !== -1 && i + 1 < args.length ? args[i + 1] : fallback;
}

if (flag("--help") || flag("-h")) {
  process.stdout.write(
    [
      "fake-claude.mjs -- drive Koden's real detector pipeline without a real limit.",
      "",
      "Usage: node scripts/fake-claude.mjs [options]   (run inside a Koden terminal)",
      "",
      "Options:",
      "  --no-shell           Arm via OSC 777 self-arm marker instead of OSC 133;C;claude.",
      "  --variant <kind>     session | weekly | opus | sonnet  (default: session).",
      '  --reset "<text>"     Inject an arbitrary reset string in place of the default',
      "                       near-future reset. Use to test custom phrasings or the",
      "                       no-match negative path.",
      "  --minutes <n>        Minutes from now for the default reset (default: 2).",
      "  --no-menu            Skip printing the auto-opened menu.",
      "  --help, -h           Show this help.",
      "",
    ].join("\r\n") + "\r\n",
  );
  process.exit(0);
}

const noShell = flag("--no-shell");
const noMenu = flag("--no-menu");
const variant = opt("--variant", "session");
const customReset = opt("--reset", null);
const minutes = Number.parseInt(opt("--minutes", "2"), 10);
const minutesAhead = Number.isFinite(minutes) && minutes > 0 ? minutes : 2;

function out(s) {
  process.stdout.write(s);
}

// --- step 1: arm the session -------------------------------------------------
// OSC 133;C;<cmd> is the shell-integration "command start" marker. The detector
// matches the basename token against its agent list ("claude"). OSC 777 is the
// Koden hook marker that self-arms even without shell preexec.
function arm() {
  if (noShell) {
    // Self-arm + set working. handle_osc777 emits Started{claude} then Working.
    out(`${ESC}]777;notify;Koden;working${ST}`);
  } else {
    // Command-start for `claude`. handle_osc133 emits Started{claude}.
    out(`${ESC}]133;C;claude${ST}`);
  }
}

// --- step 2: the limit banner ------------------------------------------------
function localTz() {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

// "<H>:<MM><am/pm>" in local time, lowercase, no leading zero on the hour --
// the shape retry_detect.rs::parse_clock expects after "resets".
function clockString(date) {
  let h = date.getHours();
  const m = date.getMinutes();
  const suffix = h >= 12 ? "pm" : "am";
  let h12 = h % 12;
  if (h12 === 0) h12 = 12;
  const mm = String(m).padStart(2, "0");
  return `${h12}:${mm}${suffix}`;
}

const MIDDLE_DOT = "·"; // U+00B7

// Per-variant lead-in. These are the EXACT modern (v2.1.x) phrasings the
// retry_detect tests assert fire -- the "you've hit your <kind> limit" prefix is
// what parse_reset matches, so the harness drives the detector's real modern
// path. The shared "(MIDDLEDOT) resets <clock> (<tz>)" clause supplies the time.
function bannerLead() {
  switch (variant) {
    case "weekly":
      // Mirrors retry_detect test `fires_on_modern_weekly_limit_banner_*`.
      return "You've hit your weekly limit";
    case "opus":
      return "You've hit your Opus limit";
    case "sonnet":
      return "You've hit your Sonnet limit";
    case "session":
    default:
      // Mirrors retry_detect test `fires_on_modern_session_limit_banner_*`.
      return "You've hit your session limit";
  }
}

function banner() {
  const tz = localTz();
  let resetClause;
  if (customReset !== null) {
    resetClause = customReset;
  } else {
    const reset = new Date(Date.now() + minutesAhead * 60_000);
    resetClause = `resets ${clockString(reset)} (${tz})`;
  }
  // Bright red lead, dim reset clause -- exercises the ANSI strip in the
  // detector (strip_ansi) on the way to the match.
  out(
    `\r\n${ESC}[1;31m${bannerLead()}${ESC}[0m ${MIDDLE_DOT} ${ESC}[2m${resetClause}${ESC}[0m\r\n`,
  );
}

// --- step 3: the auto-opened menu --------------------------------------------
const CARET = "❯"; // U+276F heavy right-pointing angle, Claude's selector

function menu() {
  if (noMenu) return;
  out("\r\nWhat do you want to do?\r\n");
  out(`${CARET} 1. Stop and wait for limit to reset\r\n`);
  out("  2. Add funds to continue with usage credits\r\n");
  out("  3. Upgrade your plan\r\n");
}

// --- step 4: echo whatever Koden injects -------------------------------------
function hex(buf) {
  return Array.from(buf)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join(" ");
}

// Render control bytes readably so the tester can recognize CR (0d), Esc (1b),
// and the bracketed-paste markers if a multiline resume ever arrives.
function printable(str) {
  return str
    .replace(/\x1b/g, "<ESC>")
    .replace(/\r/g, "<CR>")
    .replace(/\n/g, "<LF>")
    .replace(/[\x00-\x08\x0b-\x1a\x1c-\x1f]/g, (c) => `<0x${c.charCodeAt(0).toString(16).padStart(2, "0")}>`);
}

function start() {
  arm();
  banner();
  menu();
  out(
    `\r\n${ESC}[2m[harness] armed (${noShell ? "OSC 777 self-arm" : "OSC 133;C;claude"}), banner + menu printed.\r\n` +
      `[harness] waiting for Koden to inject the resume -- echoing stdin until Ctrl-C.${ESC}[0m\r\n`,
  );

  process.stdin.resume();
  process.stdin.on("data", (chunk) => {
    const text = chunk.toString("latin1");
    out(`\r\n${ESC}[36m[harness] received: "${printable(text)}"  hex: ${hex(chunk)}${ESC}[0m\r\n`);
  });
  process.stdin.on("end", () => {
    out(`\r\n${ESC}[2m[harness] stdin closed.${ESC}[0m\r\n`);
  });
}

process.on("SIGINT", () => {
  out(`\r\n${ESC}[2m[harness] SIGINT -- exiting.${ESC}[0m\r\n`);
  process.exit(0);
});

start();
