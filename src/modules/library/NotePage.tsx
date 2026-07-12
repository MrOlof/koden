import { MarkdownCode } from "@/components/ai-elements/markdown-code";
import { ScrollArea } from "@/components/ui/scroll-area";
import { currentWorkspaceEnv } from "@/modules/workspace";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { Streamdown } from "streamdown";
import { joinRoot, type PageRef } from "./lib/useLibrary";

type ReadResult =
  | { kind: "text"; content: string; size: number }
  | { kind: "binary"; size: number }
  | { kind: "toolarge"; size: number; limit: number };

type Status =
  | { kind: "loading" }
  | { kind: "ready"; content: string }
  | { kind: "unreadable"; message: string };

const components = { code: MarkdownCode };

/** The page header already shows the parsed meta; the raw YAML block would
 *  render as noise, so it is stripped from the body. */
function stripFrontmatter(md: string): string {
  const m = md.match(/^---\r?\n[\s\S]*?\r?\n---\r?\n?/);
  return m ? md.slice(m[0].length) : md;
}

export function NotePage({ page }: { page: PageRef }) {
  const [status, setStatus] = useState<Status>({ kind: "loading" });
  const fullPath = joinRoot(page.project.root, page.path);

  useEffect(() => {
    let cancelled = false;
    setStatus({ kind: "loading" });
    invoke<ReadResult>("fs_read_file", {
      path: fullPath,
      workspace: currentWorkspaceEnv(),
    })
      .then((res) => {
        if (cancelled) return;
        if (res.kind === "text") {
          setStatus({ kind: "ready", content: stripFrontmatter(res.content) });
        } else {
          setStatus({
            kind: "unreadable",
            message:
              res.kind === "binary"
                ? "Binary file. Not a page."
                : `Too large to shelve (${res.size} bytes, limit ${res.limit}).`,
          });
        }
      })
      .catch((e) => {
        if (!cancelled) setStatus({ kind: "unreadable", message: String(e) });
      });
    return () => {
      cancelled = true;
    };
  }, [fullPath]);

  return (
    <ScrollArea className="min-h-0 flex-1">
      <article className="mx-auto w-full max-w-[72ch] px-8 py-7">
        <header className="mb-6 border-b pb-4">
          <div className="truncate font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
            {page.project.name} / {page.path}
          </div>
          <h1 className="mt-1.5 font-mono text-lg leading-snug font-semibold tracking-tight">
            {page.title}
          </h1>
          {page.noteType || page.status ? (
            <div className="mt-1.5 flex items-center gap-1.5">
              {page.noteType ? (
                <span className="rounded bg-muted px-1 py-px font-mono text-[9px] uppercase tracking-wide text-muted-foreground">
                  {page.noteType}
                </span>
              ) : null}
              {page.status ? (
                <span className="font-mono text-[10px] text-muted-foreground">
                  {page.status}
                </span>
              ) : null}
            </div>
          ) : null}
        </header>
        {status.kind === "loading" ? (
          <p className="text-xs text-muted-foreground">Fetching the page…</p>
        ) : status.kind === "unreadable" ? (
          <p className="text-xs text-muted-foreground">{status.message}</p>
        ) : (
          <Streamdown
            className="select-text [&>*:first-child]:mt-0 [&>*:last-child]:mb-0"
            components={components}
          >
            {status.content}
          </Streamdown>
        )}
        {status.kind === "ready" && page.anchors.length > 0 ? (
          <footer className="mt-8 border-t pt-3">
            <div className="font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
              Anchors
            </div>
            <div className="mt-1.5 flex flex-wrap gap-1">
              {page.anchors.map((a) => (
                <span
                  key={a}
                  className="rounded border px-1.5 py-px font-mono text-[10px] text-muted-foreground"
                >
                  {a}
                </span>
              ))}
            </div>
          </footer>
        ) : null}
      </article>
    </ScrollArea>
  );
}
