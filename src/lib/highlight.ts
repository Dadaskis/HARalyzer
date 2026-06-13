import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import diff from "highlight.js/lib/languages/diff";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import python from "highlight.js/lib/languages/python";
import shell from "highlight.js/lib/languages/shell";
import powershell from "highlight.js/lib/languages/powershell";

hljs.registerLanguage("python", python);
hljs.registerLanguage("javascript", javascript);
hljs.registerLanguage("json", json);
hljs.registerLanguage("bash", bash);
hljs.registerLanguage("shell", shell);
hljs.registerLanguage("powershell", powershell);
hljs.registerLanguage("diff", diff);

const LANGUAGE_ALIASES: Record<string, string> = {
  py: "python",
  js: "javascript",
  ts: "typescript",
  jsonc: "json",
  sh: "bash",
  ps: "powershell",
  ps1: "powershell",
};

export function normalizeHighlightLanguage(language?: string): string | undefined {
  if (!language?.trim()) return undefined;
  const key = language.trim().toLowerCase();
  return LANGUAGE_ALIASES[key] ?? key;
}

export function highlightCode(code: string, language?: string): string {
  if (!code) return "";
  const lang = normalizeHighlightLanguage(language);
  if (lang && hljs.getLanguage(lang)) {
    try {
      return hljs.highlight(code, { language: lang }).value;
    } catch {
      // fall through to auto-detect
    }
  }
  try {
    return hljs.highlightAuto(code).value;
  } catch {
    return code
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }
}
