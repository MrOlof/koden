export const MAX_SLUG_LENGTH = 40;

/** Lowercase, ascii-only, hyphen separated, trimmed, capped. */
export function slugify(name: string): string {
  const collapsed = name
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  if (collapsed.length <= MAX_SLUG_LENGTH) return collapsed;
  return collapsed.slice(0, MAX_SLUG_LENGTH).replace(/-+$/g, "");
}
