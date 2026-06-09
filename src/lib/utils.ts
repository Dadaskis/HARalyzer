import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

/** Unwrap LLM responses that put the whole report inside ```markdown fences. */
export function normalizeMarkdownReport(text: string): string {
  let s = text.trim();
  if (!s) return s;

  s = s.replace(
    /^(?:here(?:'s| is)|below is|sure|certainly)[^\n#*`]{0,220}(?:report|markdown|synthesized|summary)[^\n]*:\s*\n*/i,
    ""
  );

  const firstLineBreak = s.indexOf("\n");
  if (firstLineBreak !== -1) {
    const first = s.slice(0, firstLineBreak).trim();
    if (
      first.endsWith(":") &&
      first.length < 160 &&
      !first.startsWith("#") &&
      first.toLowerCase().includes("report")
    ) {
      s = s.slice(firstLineBreak + 1).trim();
    }
  }

  const fenceMatch = s.match(/^```(?:markdown|md|text)?\s*\n([\s\S]*?)\n```\s*$/);
  if (fenceMatch) {
    s = fenceMatch[1].trim();
  }

  return s;
}
