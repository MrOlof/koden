import { describe, expect, it } from "vitest";

import {
  type DetectedLink,
  DEFAULT_LINK_TYPES,
  detectLinks,
  type LinkCategory,
  type LinkTypeConfig,
} from "./linkDetect";

function find(
  line: string,
  value: string,
  config?: LinkTypeConfig,
): DetectedLink | undefined {
  return detectLinks(line, config).find((l) => l.value === value);
}

function categories(line: string, config?: LinkTypeConfig): LinkCategory[] {
  return detectLinks(line, config).map((l) => l.category);
}

// The user's real-terminal fixture. Each line is a corpus row a Nordomatic
// admin actually saw scroll past, mixing paths, filenames, credentials,
// identity tokens and prose. The detector must classify every load-bearing
// token into the right category with the default action, and leave the
// command flags / prose / bare numbers alone.
const FIXTURE = {
  unc: "Deploying from \\\\nordic-fs01\\Share$\\IT\\Deploy\\packages now",
  // The path has spaced folders ("Program Files", "Endpoint Agent"). A trailing
  // \\bin keeps the final spaced segment unambiguous (a separator follows it),
  // demonstrating interior-space handling. See ponytail note in linkDetect.ts:
  // a spaced final segment with NOTHING after it stays at "...\\Endpoint".
  winPath:
    "Agent installed to C:\\Program Files\\Nordic Tools\\Endpoint Agent\\bin",
  winPathTrailingSpace: "Agent installed to C:\\Program Files\\Nordic Tools",
  filenames:
    "Bundled config.prod.json, NordicAgent-Setup.exe and AcmeVPN_4.12.0_x64.msi",
  winuser: "Logged in as nordic\\j.lindqvist on host",
  winuserUpper: "Token issued to NORDIC\\j.lindqvist",
  sid: "Owner SID S-1-5-21-3623811015-3361044348-30300820-1013 resolved",
  email: "Contact j.lindqvist@nordic-fabrikam.com for access",
  ips: "Reached 10.42.18.207, 172.19.0.1 and 51.144.207.18 over the tunnel",
  guids:
    "Correlation 8f4a17c2-6b3e-4d91-a0f5-2c7e9b1d4a88 / 0b9e1f3a-7c2d-1e4f-8a6b-5d3c2e1f0a9b",
  labeledSecrets:
    "CLIENT_SECRET=Xq7~9fL2mPv0RtZ.aB3cD8eH-kN6sW1yU4 API_KEY=sk-nrd-9f3c1a7e84b24d05bb6e02f7c9a1d3e0",
  jwt: "Bearer eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJzdmMtYWdlbnQifQ.3pKf9Qe2rI…",
  sha256:
    "Checksum 3F9A1C7E0D52B84A6F1029E3C7B5A8D40F2E6C1B9A4D7E0F35C8B2A1D6E4F840 verified",
  thumbprint:
    "Cert thumbprint A91F3C7E0B82D45169AE2F0C7B3D81E5C4F6A920 trusted",
  flag: "Running whoami /upn for the current user",
  prose: "the agent finished installing without any reported problems",
  bareNumber: "the exit code was 1024 today",
} as const;

describe("detectLinks — fixture, default actions (path)", () => {
  it("UNC share path -> path/open", () => {
    const link = find(FIXTURE.unc, "\\\\nordic-fs01\\Share$\\IT\\Deploy\\packages");
    expect(link?.category).toBe("path");
    expect(link?.action).toBe("open");
  });

  it("Windows drive path with interior spaced folders -> path/open", () => {
    const link = find(
      FIXTURE.winPath,
      "C:\\Program Files\\Nordic Tools\\Endpoint Agent\\bin",
    );
    expect(link?.category).toBe("path");
    expect(link?.action).toBe("open");
  });

  it("a trailing spaced folder with no following separator stops short", () => {
    // ponytail: "...\\Nordic Tools" with no separator after "Tools" is
    // indistinguishable from a path followed by the prose word "Tools"; the
    // conservative body captures "...\\Nordic" and leaves the rest alone.
    const link = find(
      FIXTURE.winPathTrailingSpace,
      "C:\\Program Files\\Nordic",
    );
    expect(link?.category).toBe("path");
    expect(link?.action).toBe("open");
  });
});

describe("detectLinks — fixture, filenames (copy, not secret)", () => {
  it("config.prod.json -> filename/copy", () => {
    const link = find(FIXTURE.filenames, "config.prod.json");
    expect(link?.category).toBe("filename");
    expect(link?.action).toBe("copy");
  });

  it("NordicAgent-Setup.exe -> filename/copy", () => {
    const link = find(FIXTURE.filenames, "NordicAgent-Setup.exe");
    expect(link?.category).toBe("filename");
  });

  it("AcmeVPN_4.12.0_x64.msi -> filename/copy (NOT secret via strong-token)", () => {
    const link = find(FIXTURE.filenames, "AcmeVPN_4.12.0_x64.msi");
    expect(link?.category).toBe("filename");
    expect(link?.action).toBe("copy");
  });

  it("all three filenames are the SAME category", () => {
    const cats = detectLinks(FIXTURE.filenames).map((l) => l.category);
    expect(cats).toEqual(["filename", "filename", "filename"]);
  });
});

describe("detectLinks — fixture, identity tokens", () => {
  it("nordic\\j.lindqvist -> winuser/copy (NOT a path)", () => {
    const link = find(FIXTURE.winuser, "nordic\\j.lindqvist");
    expect(link?.category).toBe("winuser");
    expect(link?.action).toBe("copy");
    expect(categories(FIXTURE.winuser)).not.toContain("path");
  });

  it("uppercase NORDIC\\j.lindqvist -> winuser/copy", () => {
    const link = find(FIXTURE.winuserUpper, "NORDIC\\j.lindqvist");
    expect(link?.category).toBe("winuser");
  });

  it("Windows SID -> sid/copy", () => {
    const link = find(
      FIXTURE.sid,
      "S-1-5-21-3623811015-3361044348-30300820-1013",
    );
    expect(link?.category).toBe("sid");
    expect(link?.action).toBe("copy");
  });

  it("UPN-style email -> email/copy", () => {
    const link = find(FIXTURE.email, "j.lindqvist@nordic-fabrikam.com");
    expect(link?.category).toBe("email");
    expect(link?.action).toBe("copy");
  });
});

describe("detectLinks — fixture, IPv4", () => {
  it("detects all three IPs as ip/copy", () => {
    const ips = detectLinks(FIXTURE.ips).filter((l) => l.category === "ip");
    expect(ips.map((l) => l.value)).toEqual([
      "10.42.18.207",
      "172.19.0.1",
      "51.144.207.18",
    ]);
    expect(ips.every((l) => l.action === "copy")).toBe(true);
  });
});

describe("detectLinks — fixture, GUIDs", () => {
  it("detects both any-version UUIDs as guid/copy", () => {
    const guids = detectLinks(FIXTURE.guids).filter(
      (l) => l.category === "guid",
    );
    expect(guids.map((l) => l.value)).toEqual([
      "8f4a17c2-6b3e-4d91-a0f5-2c7e9b1d4a88",
      "0b9e1f3a-7c2d-1e4f-8a6b-5d3c2e1f0a9b",
    ]);
    expect(guids.every((l) => l.action === "copy")).toBe(true);
  });
});

describe("detectLinks — fixture, secrets", () => {
  it("CLIENT_SECRET=... -> secret/copy (value only)", () => {
    const link = find(
      FIXTURE.labeledSecrets,
      "Xq7~9fL2mPv0RtZ.aB3cD8eH-kN6sW1yU4",
    );
    expect(link?.category).toBe("secret");
    expect(link?.action).toBe("copy");
  });

  it("API_KEY=sk-... -> secret/copy", () => {
    const link = find(
      FIXTURE.labeledSecrets,
      "sk-nrd-9f3c1a7e84b24d05bb6e02f7c9a1d3e0",
    );
    expect(link?.category).toBe("secret");
  });

  it("JWT -> secret/copy", () => {
    const jwt =
      "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJzdmMtYWdlbnQifQ.3pKf9Qe2rI";
    const link = find(FIXTURE.jwt, jwt);
    expect(link?.category).toBe("secret");
    expect(link?.action).toBe("copy");
  });

  it("SHA256 digest -> secret/copy", () => {
    const sha =
      "3F9A1C7E0D52B84A6F1029E3C7B5A8D40F2E6C1B9A4D7E0F35C8B2A1D6E4F840";
    const link = find(FIXTURE.sha256, sha);
    expect(link?.category).toBe("secret");
  });

  it("SHA1 cert thumbprint -> secret/copy", () => {
    const tp = "A91F3C7E0B82D45169AE2F0C7B3D81E5C4F6A920";
    const link = find(FIXTURE.thumbprint, tp);
    expect(link?.category).toBe("secret");
    expect(link?.action).toBe("copy");
  });
});

describe("detectLinks — fixture, negatives", () => {
  it("command flag /upn is NOT a path", () => {
    expect(detectLinks(FIXTURE.flag)).toEqual([]);
  });

  it.each(["/upn", "/user", "/svc", "/i", "/qn"])(
    "lone flag %s is never a path",
    (flag) => {
      expect(
        detectLinks(`run tool ${flag} now`).filter((l) => l.category === "path"),
      ).toEqual([]);
    },
  );

  it("ordinary prose yields nothing", () => {
    expect(detectLinks(FIXTURE.prose)).toEqual([]);
  });

  it("a bare number yields nothing", () => {
    expect(detectLinks(FIXTURE.bareNumber)).toEqual([]);
  });
});

describe("detectLinks — POSIX path segment rule", () => {
  it("absolute path needs >=2 segments", () => {
    expect(
      detectLinks("/etc/hosts").filter((l) => l.category === "path"),
    ).toHaveLength(1);
    expect(
      detectLinks("/etc/hosts")[0]?.value,
    ).toBe("/etc/hosts");
  });

  it("a lone /word is a flag, not a path", () => {
    expect(detectLinks("pass /quiet to it")).toEqual([]);
  });

  it("anchored ~ ./ ../ single-segment paths still match", () => {
    expect(detectLinks("~/notes")[0]?.value).toBe("~/notes");
    expect(detectLinks("./build")[0]?.value).toBe("./build");
    expect(detectLinks("../out")[0]?.value).toBe("../out");
  });
});

describe("detectLinks — config gates the action", () => {
  const off = (cat: LinkCategory): LinkTypeConfig => ({
    ...DEFAULT_LINK_TYPES,
    [cat]: "off",
  });

  it("'off' suppresses a category entirely", () => {
    expect(find(FIXTURE.ips, "10.42.18.207", off("ip"))).toBeUndefined();
    expect(find(FIXTURE.email, "j.lindqvist@nordic-fabrikam.com", off("email"))).toBeUndefined();
  });

  it("turning paths off does not leak the value into another category", () => {
    const links = detectLinks(FIXTURE.winPath, off("path"));
    expect(links.some((l) => l.value.startsWith("C:\\"))).toBe(false);
  });

  it("'open' vs 'copy' flips the action for the same line", () => {
    const copyCfg: LinkTypeConfig = { ...DEFAULT_LINK_TYPES, path: "copy" };
    const value = "C:\\Program Files\\Nordic Tools\\Endpoint Agent\\bin";
    const openLink = find(FIXTURE.winPath, value);
    const copyLink = find(FIXTURE.winPath, value, copyCfg);
    expect(openLink?.action).toBe("open");
    expect(copyLink?.action).toBe("copy");
  });

  it("filename can be flipped to open", () => {
    const openCfg: LinkTypeConfig = { ...DEFAULT_LINK_TYPES, filename: "open" };
    expect(find(FIXTURE.filenames, "config.prod.json", openCfg)?.action).toBe(
      "open",
    );
  });
});

describe("detectLinks — URL exclusion + structure", () => {
  it("never claims an http/https URL (WebLinksAddon owns those)", () => {
    expect(detectLinks("visit https://example.com/a/b")).toEqual([]);
  });

  it("does not double-claim overlapping spans", () => {
    const links = detectLinks(FIXTURE.guids);
    for (let i = 1; i < links.length; i++) {
      expect(links[i].start).toBeGreaterThanOrEqual(links[i - 1].end);
    }
  });

  it("returns matches sorted by start offset", () => {
    const links = detectLinks(FIXTURE.ips);
    const starts = links.map((l) => l.start);
    expect([...starts].sort((a, b) => a - b)).toEqual(starts);
  });

  it("empty line yields nothing", () => {
    expect(detectLinks("")).toEqual([]);
  });

  it("range maps back to the matched substring", () => {
    const line = FIXTURE.email;
    const link = find(line, "j.lindqvist@nordic-fabrikam.com");
    expect(link && line.slice(link.start, link.end)).toBe(
      "j.lindqvist@nordic-fabrikam.com",
    );
  });
});
