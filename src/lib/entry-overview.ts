import type { HeaderPair, HarEntryDetail } from "@/lib/types";
import { classifyEntryResource, RESOURCE_FILTERS } from "@/lib/entry-filters";

export function getHeader(headers: HeaderPair[], name: string): string | undefined {
  const lower = name.toLowerCase();
  return headers.find((h) => h.name.toLowerCase() === lower)?.value;
}

export function getHeaders(headers: HeaderPair[], name: string): string[] {
  const lower = name.toLowerCase();
  return headers.filter((h) => h.name.toLowerCase() === lower).map((h) => h.value);
}

export function bodyByteLength(body: string): number {
  if (!body) return 0;
  return new TextEncoder().encode(body).length;
}

export function statusCategory(status: number): {
  label: string;
  tone: "success" | "warning" | "error" | "neutral";
} {
  if (status === 0) return { label: "Failed / blocked", tone: "error" };
  if (status >= 200 && status < 300) return { label: "Success", tone: "success" };
  if (status >= 300 && status < 400) return { label: "Redirect", tone: "warning" };
  if (status >= 400 && status < 500) return { label: "Client error", tone: "error" };
  if (status >= 500) return { label: "Server error", tone: "error" };
  return { label: "Informational", tone: "neutral" };
}

export interface UrlParts {
  protocol: string;
  host: string;
  pathname: string;
  search: string;
  query: { key: string; value: string }[];
}

export function parseEntryUrl(url: string): UrlParts | null {
  try {
    const parsed = new URL(url);
    const query = [...parsed.searchParams.entries()].map(([key, value]) => ({ key, value }));
    return {
      protocol: parsed.protocol.replace(":", ""),
      host: parsed.host,
      pathname: parsed.pathname || "/",
      search: parsed.search,
      query,
    };
  } catch {
    return null;
  }
}

export function resourceTypeLabel(entry: HarEntryDetail["summary"]): string {
  const id = classifyEntryResource(entry);
  return RESOURCE_FILTERS.find((f) => f.id === id)?.label ?? id;
}

export interface NotableHeader {
  label: string;
  value: string;
}

export function collectNotableHeaders(entry: HarEntryDetail): NotableHeader[] {
  const items: NotableHeader[] = [];
  const { request_headers: req, response_headers: res } = entry;

  const reqType = getHeader(req, "content-type");
  const resType = getHeader(res, "content-type") ?? entry.summary.mime_type;
  if (reqType) items.push({ label: "Request Content-Type", value: reqType });
  if (resType) items.push({ label: "Response Content-Type", value: resType });

  const reqLen = getHeader(req, "content-length");
  const resLen = getHeader(res, "content-length");
  if (reqLen) items.push({ label: "Request Content-Length", value: reqLen });
  if (resLen) items.push({ label: "Response Content-Length", value: resLen });

  const cache = getHeader(res, "cache-control");
  if (cache) items.push({ label: "Cache-Control", value: cache });

  const etag = getHeader(res, "etag");
  if (etag) items.push({ label: "ETag", value: etag });

  const server = getHeader(res, "server");
  if (server) items.push({ label: "Server", value: server });

  const encoding = getHeader(res, "content-encoding");
  if (encoding) items.push({ label: "Content-Encoding", value: encoding });

  const cookies = getHeaders(res, "set-cookie");
  if (cookies.length > 0) {
    items.push({
      label: "Set-Cookie",
      value: `${cookies.length} cookie${cookies.length === 1 ? "" : "s"}`,
    });
  }

  const cookie = getHeader(req, "cookie");
  if (cookie) {
    const count = cookie.split(";").filter(Boolean).length;
    items.push({ label: "Cookie", value: `${count} sent` });
  }

  if (getHeader(req, "authorization")) {
    items.push({ label: "Authorization", value: "Present (redacted)" });
  }

  const location = getHeader(res, "location");
  if (location) items.push({ label: "Location", value: location });

  return items;
}

export function bodyPreview(body: string, max = 280): string | null {
  const trimmed = body.trim();
  if (!trimmed) return null;
  if (trimmed.length <= max) return trimmed;
  return `${trimmed.slice(0, max)}…`;
}
