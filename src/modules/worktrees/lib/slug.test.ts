import { describe, expect, it } from "vitest";
import { MAX_SLUG_LENGTH, slugify } from "./slug";

describe("slugify", () => {
  it("lowercases and hyphenates runs of non-alphanumerics", () => {
    expect(slugify("Fix Login  Bug!")).toBe("fix-login-bug");
    expect(slugify("feat/Payments v2")).toBe("feat-payments-v2");
  });

  it("trims leading and trailing separators", () => {
    expect(slugify("  --hello world--  ")).toBe("hello-world");
    expect(slugify("...")).toBe("");
  });

  it("drops accents to ascii", () => {
    expect(slugify("Ångström café")).toBe("angstrom-cafe");
  });

  it("caps at the max length without a dangling hyphen", () => {
    const long = `${"a".repeat(30)} ${"b".repeat(30)}`;
    const out = slugify(long);
    expect(out.length).toBeLessThanOrEqual(MAX_SLUG_LENGTH);
    expect(out.endsWith("-")).toBe(false);
    expect(out).toBe(`${"a".repeat(30)}-${"b".repeat(9)}`);
  });

  it("returns empty for empty input", () => {
    expect(slugify("")).toBe("");
  });
});
