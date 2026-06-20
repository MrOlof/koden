// Pure detection core for the smart terminal link providers. Given a single
// line of terminal text it returns the spans that should become clickable, each
// carrying a resolved ACTION ("copy" reveals a clipboard affordance; "open"
// reveals a path in the file manager / opens a URL). The set of categories and
// their per-type action is configurable from Settings; this module owns the
// shapes and the claim-ordering, the caller owns the wiring.
//
// All ranges are 0-based half-open [start, end) character offsets into the line;
// the caller maps them onto xterm's 1-based buffer columns.
//
// Two hard rules drive every regex here:
//   1. Never claim a range the WebLinksAddon already owns (http/https URLs).
//   2. The COPY shapes use TIGHT allowlists so we never offer to copy ordinary
//      prose. A false positive that silently copies the wrong thing is worse
//      than missing a link, so the bar is "unambiguous token shape".

// The eight detection categories. Order here is irrelevant; claim priority is
// enforced in detectLinks. Adding a category later is additive: give it an id,
// a default action, and a claim step.
export type LinkCategory =
  | "path"
  | "filename"
  | "ip"
  | "email"
  | "guid"
  | "secret"
  | "sid"
  | "winuser";

export type LinkAction = "off" | "copy" | "open";

export type LinkTypeConfig = Record<LinkCategory, LinkAction>;

// The baseline the user asked for: "not too much, the basics." Paths open;
// everything else is a high-signal copyable token.
export const DEFAULT_LINK_TYPES: LinkTypeConfig = {
  path: "open",
  filename: "copy",
  ip: "copy",
  email: "copy",
  guid: "copy",
  secret: "copy",
  sid: "copy",
  winuser: "copy",
};

export const LINK_CATEGORY_LABELS: Record<LinkCategory, string> = {
  path: "File paths",
  filename: "Filenames",
  ip: "IP addresses",
  email: "Email addresses",
  guid: "GUIDs / UUIDs",
  secret: "Secrets & tokens",
  sid: "Windows SIDs",
  winuser: "Domain users",
};

// Stable display order for the Settings UI: most-broadly-useful first.
export const LINK_CATEGORY_ORDER: LinkCategory[] = [
  "path",
  "filename",
  "ip",
  "email",
  "guid",
  "secret",
  "sid",
  "winuser",
];

export type DetectedLink = {
  category: LinkCategory;
  action: "copy" | "open";
  start: number;
  end: number;
  value: string;
};

// Curated filename extension allowlist. A bare name.ext is offered only for
// these so .exe/.msi/.json read consistently and prose like "etc." never bites.
const FILENAME_EXTS = [
  "exe",
  "msi",
  "dll",
  "json",
  "xml",
  "config",
  "ps1",
  "psm1",
  "log",
  "txt",
  "csv",
  "yaml",
  "yml",
  "zip",
  "cab",
  "sys",
  "inf",
  "reg",
  "cer",
  "pfx",
  "md",
  "sh",
  "bat",
  "cmd",
  "ini",
  "toml",
  "env",
  "cs",
  "ts",
  "tsx",
  "js",
] as const;

const KNOWN_EXT_SET = new Set<string>(FILENAME_EXTS);
const EXT_ALT = FILENAME_EXTS.join("|");

// http/https (and bare www.) — mirrors what WebLinksAddon claims so we can
// carve those ranges out before running our own detection.
const URL_RE = /\b(?:https?:\/\/|www\.)[^\s"'<>]+/gi;

// --- path category --------------------------------------------------------
// Windows drive path: C:\foo\bar or C:/foo/bar. A space is treated as
// path-internal only when more text then another separator follows it, so
// "C:\Program Files\Nordic Tools\x" is one path but "C:\temp and then go" stops
// at "C:\temp". Stops at the shell-quote / bracket chars that never appear
// mid-path.
// ponytail: a trailing spaced folder with no following separator (e.g. the
// final "...\Endpoint Agent") is undecidable from prose ("...\Endpoint" then a
// word) and is intentionally NOT absorbed; over-capturing prose would point a
// reveal at the wrong path, which the project's "false positive is worse than a
// miss" bar rejects.
const PATH_SEG = "[^\\s\"'`<>|]";
const WIN_DRIVE_RE = new RegExp(
  `\\b[A-Za-z]:[\\\\/](?:${PATH_SEG}| (?=${PATH_SEG}*[\\\\/]))*${PATH_SEG}*`,
  "g",
);
// UNC path: \\server\share\... (at least one segment after the host).
const UNC_RE = /\\\\[^\s"'`<>|]+/g;
// $env:VAR\.. PowerShell env-prefixed path.
const ENV_PREFIX_RE = /\$env:[A-Za-z_][A-Za-z0-9_]*[\\/][^\s"'`<>|]*/gi;
// POSIX-ish anchored paths: ~/x, ./x, ../x. A separator is required so a bare
// "." or "~" is never a link.
const POSIX_ANCHORED_RE = /(?:~|\.{1,2})\/[A-Za-z0-9._\-/+@~]+/g;
// Absolute POSIX path with >=2 segments (/a/b...). The >=2 requirement is what
// stops command flags like /upn /qn /i being claimed as paths.
const POSIX_ABS_RE = /\/[A-Za-z0-9._\-+@]+(?:\/[A-Za-z0-9._\-+@]+)+\/?/g;
// Relative backslash path: >=2 backslash-separated segments (src\Components\x)
// OR a single-segment-then-name ending in a known extension (dir\file.ts). The
// single-backslash, no-known-ext case is left to winuser.
const REL_BACKSLASH_RE = new RegExp(
  `\\b[A-Za-z0-9._\\-+@]+\\\\[A-Za-z0-9._\\-+@\\\\]*(?:[A-Za-z0-9._\\-+@]+\\\\[A-Za-z0-9._\\-+@]+|[A-Za-z0-9._\\-+@]+\\.(?:${EXT_ALT}))\\b`,
  "gi",
);

// --- filename category ----------------------------------------------------
// A bare name.ext for the curated extension allowlist. The name may itself
// contain dots (config.prod.json) and the usual filename glyphs.
const FILENAME_RE = new RegExp(
  `\\b[A-Za-z0-9][A-Za-z0-9._\\-+]*\\.(?:${EXT_ALT})\\b`,
  "gi",
);

// --- ip category ----------------------------------------------------------
const OCTET = "(?:25[0-5]|2[0-4]\\d|1\\d\\d|[1-9]?\\d)";
const IPV4_RE = new RegExp(
  `\\b${OCTET}(?:\\.${OCTET}){3}(?::\\d{1,5})?\\b`,
  "g",
);

// --- email category -------------------------------------------------------
const EMAIL_RE =
  /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?)*\.[A-Za-z]{2,}\b/g;

// --- guid category --------------------------------------------------------
// ANY-version UUID 8-4-4-4-12 hex (broadened from v4-only).
const GUID_RE =
  /\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b/g;

// --- sid category ---------------------------------------------------------
// Windows SID: S-1-(\d+-)+\d+
const SID_RE = /\bS-1-(?:\d+-)+\d+\b/g;

// --- secret category ------------------------------------------------------
const JWT_RE = /\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g;
// Known vendor token shapes. Each is high-signal.
const VENDOR_RE =
  /\b(?:ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|sk-[A-Za-z0-9_-]{20,}|AKIA[A-Z0-9]{16}|xox[baprs]-[A-Za-z0-9-]{10,}|glpat-[A-Za-z0-9_-]{20,})\b/g;
// Pure hex of exactly 32 (md5), 40 (sha1) or 64 (sha256). The negative
// look-arounds keep it from biting into a longer hex run or an identifier.
const HEX_RE =
  /(?<![0-9a-fA-F])(?:[0-9a-fA-F]{64}|[0-9a-fA-F]{40}|[0-9a-fA-F]{32})(?![0-9a-fA-F])/g;
// Label-anchored credentials: keyword [:=] value. Low-false-positive way to
// catch usernames/passwords with no recognizable shape. Bare `key`/`token` are
// intentionally NOT keywords (they appear constantly in prose); the qualified
// forms (api_key, access_key, client_secret, …) and the strong-token rule
// cover the real cases. Only the captured value becomes the link.
const LABEL_RE =
  /(?:^|[^A-Za-z0-9_])(?:user(?:name)?|login|account|pass(?:wd|word)?|pwd|secret|api[-_ ]?key|access[-_ ]?key|client[-_ ]?secret|tenant[-_ ]?id|client[-_ ]?id)\s*[:=]\s*(\S+)/gi;
const MIN_LABEL_VALUE = 3;
// Standalone high-entropy token: length >= 16 with lower+upper+digit+symbol.
// Runs last and skips anything filename-shaped or already claimed.
const STRONG_TOKEN_RE = /[^\s"'`]{16,}/g;
const STRONG_MIN_LEN = 16;

const MAX_LINKS_PER_LINE = 24;

type Span = { start: number; end: number };

function collectUrlSpans(line: string): Span[] {
  const spans: Span[] = [];
  URL_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  // biome-ignore lint/suspicious/noAssignInExpressions: standard exec loop
  while ((m = URL_RE.exec(line)) !== null) {
    spans.push({ start: m.index, end: m.index + m[0].length });
  }
  return spans;
}

function overlapsAny(start: number, end: number, spans: Span[]): boolean {
  for (const s of spans) {
    if (start < s.end && end > s.start) return true;
  }
  return false;
}

// Some patterns greedily eat a trailing punctuation char that is really
// sentence punctuation, not part of the value. Trim a single trailing run.
function trimTrailing(value: string): string {
  return value.replace(/[.,;:)\]}>'"]+$/, "");
}

// Strip one layer of matching surrounding quotes from a credential value.
function unquote(value: string): string {
  if (value.length >= 2) {
    const first = value[0];
    const last = value[value.length - 1];
    if ((first === '"' || first === "'" || first === "`") && first === last) {
      return value.slice(1, -1);
    }
  }
  return value;
}

// Unquote BEFORE trimming trailing punctuation: a quoted "...y" would otherwise
// lose its closing quote to trimTrailing and the unquote pair check would fail.
function cleanValue(raw: string): string {
  return trimTrailing(unquote(raw));
}

function isStrongToken(value: string): boolean {
  if (value.length < STRONG_MIN_LEN) return false;
  return (
    /[a-z]/.test(value) &&
    /[A-Z]/.test(value) &&
    /[0-9]/.test(value) &&
    /[^A-Za-z0-9]/.test(value)
  );
}

// True when the value looks like a bare name.ext on the curated allowlist. The
// strong-token (secret) rule excludes these so AcmeVPN_4.12.0_x64.msi is a
// filename, never a secret.
function isFilenameShaped(value: string): boolean {
  const m = /\.([A-Za-z0-9]+)$/.exec(value);
  if (!m) return false;
  return KNOWN_EXT_SET.has(m[1].toLowerCase());
}

type Accum = {
  claimed: Span[];
  out: DetectedLink[];
  config: LinkTypeConfig;
};

// Resolve a category to its effective action; "off" returns null so the caller
// claims the range (preventing a lower-priority category from re-detecting it)
// but emits nothing.
function actionFor(
  category: LinkCategory,
  config: LinkTypeConfig,
): "copy" | "open" | null {
  const a = config[category];
  return a === "off" ? null : a;
}

function claim(acc: Accum, category: LinkCategory, span: Span, value: string) {
  acc.claimed.push(span);
  const action = actionFor(category, acc.config);
  if (!action) return;
  acc.out.push({ category, action, start: span.start, end: span.end, value });
}

function pushMatches(
  re: RegExp,
  category: LinkCategory,
  line: string,
  acc: Accum,
  trim: boolean,
): void {
  re.lastIndex = 0;
  let m: RegExpExecArray | null;
  // biome-ignore lint/suspicious/noAssignInExpressions: standard exec loop
  while ((m = re.exec(line)) !== null) {
    let value = m[0];
    let end = m.index + value.length;
    if (trim) {
      const trimmed = trimTrailing(value);
      end -= value.length - trimmed.length;
      value = trimmed;
    }
    const start = m.index;
    if (end <= start) continue;
    if (overlapsAny(start, end, acc.claimed)) continue;
    claim(acc, category, { start, end }, value);
  }
}

// Label-anchored values: the value is capture group 1, so the range covers the
// value only (not the keyword/separator). Quotes are stripped and the range is
// tightened to the unquoted token. Always the "secret" category.
function pushLabeledValues(line: string, acc: Accum): void {
  LABEL_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  // biome-ignore lint/suspicious/noAssignInExpressions: standard exec loop
  while ((m = LABEL_RE.exec(line)) !== null) {
    const raw = m[1];
    if (!raw) continue;
    const rawStart = m.index + m[0].length - raw.length;
    const value = cleanValue(raw);
    if (value.length < MIN_LABEL_VALUE) continue;
    const offset = raw.indexOf(value);
    const start = rawStart + (offset >= 0 ? offset : 0);
    const end = start + value.length;
    if (overlapsAny(start, end, acc.claimed)) continue;
    claim(acc, "secret", { start, end }, value);
  }
}

// DOMAIN\user: exactly ONE backslash, both parts word-ish, NOT a known-ext
// filename and NOT a >=2-segment path. Paths (>=2 backslashes / known ext) are
// claimed earlier, so by the time this runs a surviving single-backslash token
// is a domain login.
const WINUSER_RE = /\b[A-Za-z0-9._-]+\\[A-Za-z0-9._-]+\b/g;

function pushWinUser(line: string, acc: Accum): void {
  WINUSER_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  // biome-ignore lint/suspicious/noAssignInExpressions: standard exec loop
  while ((m = WINUSER_RE.exec(line)) !== null) {
    const value = m[0];
    const start = m.index;
    const end = start + value.length;
    if (overlapsAny(start, end, acc.claimed)) continue;
    if (isFilenameShaped(value)) continue;
    claim(acc, "winuser", { start, end }, value);
  }
}

// Unlabeled strong tokens. Runs last so every shaped category claims first; a
// path-, filename- or guid-shaped string is never reclassified as a secret.
function pushStrongTokens(line: string, acc: Accum): void {
  STRONG_TOKEN_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  // biome-ignore lint/suspicious/noAssignInExpressions: standard exec loop
  while ((m = STRONG_TOKEN_RE.exec(line)) !== null) {
    const raw = m[0];
    const value = cleanValue(raw);
    if (!isStrongToken(value)) continue;
    if (isFilenameShaped(value)) continue;
    const start = m.index + raw.indexOf(value);
    const end = start + value.length;
    if (end <= start) continue;
    if (overlapsAny(start, end, acc.claimed)) continue;
    claim(acc, "secret", { start, end }, value);
  }
}

export function detectLinks(
  line: string,
  config: LinkTypeConfig = DEFAULT_LINK_TYPES,
): DetectedLink[] {
  if (!line) return [];
  // URLs are owned by WebLinksAddon: seed them as claimed so no category
  // re-claims a URL substring.
  const acc: Accum = { claimed: collectUrlSpans(line), out: [], config };

  // Claim most-specific first so nothing double-claims.
  // 1. secret: JWT / vendor / labeled creds (the highest-signal shapes).
  pushMatches(JWT_RE, "secret", line, acc, false);
  pushMatches(VENDOR_RE, "secret", line, acc, false);
  pushLabeledValues(line, acc);
  // 2. guid (before hex: a GUID's hex runs must not be eaten by HEX_RE).
  pushMatches(GUID_RE, "guid", line, acc, false);
  // 3. sid.
  pushMatches(SID_RE, "sid", line, acc, false);
  // 4. email (before filename/path so user@host.tld is not split).
  pushMatches(EMAIL_RE, "email", line, acc, false);
  // 5. long pure hex digests (secret).
  pushMatches(HEX_RE, "secret", line, acc, false);
  // 6. filename: bare name.ext on the curated allowlist.
  pushMatches(FILENAME_RE, "filename", line, acc, true);
  // 7. paths: drive, UNC, env-prefixed, relative-backslash, posix anchored,
  //    posix absolute (>=2 segments).
  pushMatches(WIN_DRIVE_RE, "path", line, acc, true);
  pushMatches(UNC_RE, "path", line, acc, true);
  pushMatches(ENV_PREFIX_RE, "path", line, acc, true);
  pushMatches(REL_BACKSLASH_RE, "path", line, acc, true);
  pushMatches(POSIX_ANCHORED_RE, "path", line, acc, true);
  pushMatches(POSIX_ABS_RE, "path", line, acc, true);
  // 8. winuser: surviving single-backslash DOMAIN\user.
  pushWinUser(line, acc);
  // 9. ip.
  pushMatches(IPV4_RE, "ip", line, acc, false);
  // 10. standalone strong token (last; skips filename-shaped + claimed).
  pushStrongTokens(line, acc);

  acc.out.sort((a, b) => a.start - b.start);
  return acc.out.length > MAX_LINKS_PER_LINE
    ? acc.out.slice(0, MAX_LINKS_PER_LINE)
    : acc.out;
}
