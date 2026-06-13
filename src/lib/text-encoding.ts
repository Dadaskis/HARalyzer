/** Common UTF-8-as-Latin-1 mojibake (e.g. Cyrillic shown as "РџСЂ…"). */
export function fixUtf8Mojibake(text: string): string {
  if (!text || !/[\u0080-\u00ff]/.test(text)) return text;

  const cyrillicBefore = countCyrillic(text);
  if (cyrillicBefore > 2) return text;

  try {
    const bytes = Uint8Array.from(text, (c) => c.charCodeAt(0) & 0xff);
    const decoded = new TextDecoder("utf-8", { fatal: false }).decode(bytes);
    const cyrillicAfter = countCyrillic(decoded);
    if (cyrillicAfter > cyrillicBefore && decoded.length > 0) {
      return decoded;
    }
  } catch {
    /* keep original */
  }
  return text;
}

function countCyrillic(text: string): number {
  const m = text.match(/[\u0400-\u04FF]/g);
  return m ? m.length : 0;
}

/** Decode literal \\uXXXX sequences when JSON.parse fails. */
export function decodeJsonUnicodeLiterals(text: string): string {
  return text.replace(/\\u([0-9a-fA-F]{4})/g, (_, hex) =>
    String.fromCharCode(parseInt(hex, 16))
  );
}
