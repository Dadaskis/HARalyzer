import { decodeJsonUnicodeLiterals, fixUtf8Mojibake } from "@/lib/text-encoding";

const TRUNCATION_SUFFIX = /\s*…\s*\[truncated[^\]]*\]\s*$/;

/** Strip HARalyzer truncation marker before attempting JSON parse. */
export function stripBodyTruncation(text: string): string {
  return text.replace(TRUNCATION_SUFFIX, "").trimEnd();
}

export function isTruncatedBody(text: string): boolean {
  return TRUNCATION_SUFFIX.test(text) || text.includes("[truncated");
}

export function looksLikeJson(text: string): boolean {
  const trimmed = stripBodyTruncation(text.trim());
  return trimmed.startsWith("{") || trimmed.startsWith("[");
}

function prepareJsonCandidate(text: string): string {
  return fixUtf8Mojibake(stripBodyTruncation(text.trim()));
}

export function tryFormatJson(
  text: string
): { ok: true; formatted: string } | { ok: false; display?: string } {
  let candidate = prepareJsonCandidate(text);
  if (!candidate.startsWith("{") && !candidate.startsWith("[")) {
    return { ok: false };
  }
  try {
    const parsed = JSON.parse(candidate);
    return { ok: true, formatted: JSON.stringify(parsed, null, 2) };
  } catch {
    candidate = decodeJsonUnicodeLiterals(candidate);
    try {
      const parsed = JSON.parse(candidate);
      return { ok: true, formatted: JSON.stringify(parsed, null, 2) };
    } catch {
      return { ok: false, display: fixUtf8Mojibake(candidate) };
    }
  }
}

export function formatJsonForDisplay(text: string, mimeType?: string): {
  displayText: string;
  isJson: boolean;
} {
  const formatted = tryFormatJson(text);
  if (formatted.ok) {
    return { displayText: formatted.formatted, isJson: true };
  }

  const mimeJson =
    !!mimeType &&
    (mimeType.includes("json") || mimeType.includes("+json") || mimeType.includes("javascript"));

  if (mimeJson && looksLikeJson(text)) {
    const raw = fixUtf8Mojibake(stripBodyTruncation(text.trim()));
    if ("display" in formatted && formatted.display) {
      return { displayText: formatted.display, isJson: true };
    }
    return { displayText: raw, isJson: true };
  }

  return { displayText: fixUtf8Mojibake(text), isJson: false };
}
