import type { HarEntryDetail } from "@/lib/types";

export function entryAsCurl(entry: HarEntryDetail): string {
  const method = entry.summary.method.toUpperCase();
  const url = entry.summary.url;
  const lines = [`curl -X ${method} '${url.replace(/'/g, "'\\''")}'`];

  for (const h of entry.request_headers ?? []) {
    const lower = h.name.toLowerCase();
    if (lower === "content-length" || lower === "host") continue;
    lines.push(`  -H '${h.name}: ${h.value.replace(/'/g, "'\\''")}'`);
  }

  if (entry.request_body.trim() && method !== "GET" && method !== "HEAD") {
    const body = entry.request_body.replace(/'/g, "'\\''");
    lines.push(`  --data-raw '${body}'`);
  }

  return lines.join(" \\\n");
}

export function entryAsHarSource(entry: HarEntryDetail): string {
  return JSON.stringify(
    {
      index: entry.summary.index,
      method: entry.summary.method,
      url: entry.summary.url,
      status: entry.summary.status,
      request_headers: entry.request_headers,
      response_headers: entry.response_headers,
      request_body: entry.request_body,
      response_body: entry.response_body,
    },
    null,
    2
  );
}

export function entryHeadersText(entry: HarEntryDetail, side: "request" | "response" | "both"): string {
  const parts: string[] = [];
  if (side === "request" || side === "both") {
    parts.push("Request headers:");
    for (const h of entry.request_headers ?? []) {
      parts.push(`${h.name}: ${h.value}`);
    }
  }
  if (side === "response" || side === "both") {
    if (parts.length) parts.push("");
    parts.push("Response headers:");
    for (const h of entry.response_headers ?? []) {
      parts.push(`${h.name}: ${h.value}`);
    }
  }
  return parts.join("\n");
}

export function entryUrlLine(entry: HarEntryDetail): string {
  return `${entry.summary.method} ${entry.summary.url}`;
}
