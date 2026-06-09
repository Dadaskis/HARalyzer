import type { HarEntrySummary } from "@/lib/types";

export const RESOURCE_FILTERS = [
  { id: "all", label: "All" },
  { id: "doc", label: "Doc" },
  { id: "xhr", label: "XHR" },
  { id: "js", label: "JS" },
  { id: "css", label: "CSS" },
  { id: "img", label: "Img" },
  { id: "media", label: "Media" },
  { id: "font", label: "Font" },
  { id: "ws", label: "WS" },
  { id: "wasm", label: "Wasm" },
  { id: "other", label: "Other" },
] as const;

export type ResourceFilterId = (typeof RESOURCE_FILTERS)[number]["id"];

function mapChromeResourceType(resourceType: string): ResourceFilterId {
  switch (resourceType.toLowerCase()) {
    case "document":
      return "doc";
    case "xhr":
    case "fetch":
      return "xhr";
    case "script":
      return "js";
    case "stylesheet":
      return "css";
    case "image":
      return "img";
    case "media":
      return "media";
    case "font":
      return "font";
    case "websocket":
      return "ws";
    case "wasm":
      return "wasm";
    default:
      return "other";
  }
}

function inferResourceType(entry: HarEntrySummary): ResourceFilterId {
  const mime = entry.mime_type.toLowerCase();
  const url = entry.url.toLowerCase();

  if (mime.includes("html") || url.endsWith(".html") || url.endsWith(".htm")) {
    return "doc";
  }
  if (
    entry.is_javascript ||
    mime.includes("javascript") ||
    mime.includes("ecmascript") ||
    url.endsWith(".js") ||
    url.endsWith(".mjs")
  ) {
    return "js";
  }
  if (mime.includes("css") || url.endsWith(".css")) {
    return "css";
  }
  if (mime.startsWith("image/") || /\.(png|jpe?g|gif|webp|svg|ico|avif)(\?|$)/.test(url)) {
    return "img";
  }
  if (
    mime.startsWith("video/") ||
    mime.startsWith("audio/") ||
    /\.(mp4|webm|mp3|ogg|wav)(\?|$)/.test(url)
  ) {
    return "media";
  }
  if (
    mime.includes("font") ||
    mime.includes("woff") ||
    /\.(woff2?|ttf|otf|eot)(\?|$)/.test(url)
  ) {
    return "font";
  }
  if (mime.includes("wasm") || url.endsWith(".wasm")) {
    return "wasm";
  }
  if (
    url.startsWith("ws://") ||
    url.startsWith("wss://") ||
    mime.includes("websocket")
  ) {
    return "ws";
  }
  if (
    mime.includes("json") ||
    mime.includes("xml") ||
    mime.includes("grpc") ||
    mime.includes("x-www-form-urlencoded") ||
    (entry.method !== "GET" &&
      !mime.includes("html") &&
      !mime.startsWith("image/") &&
      !mime.includes("css") &&
      !mime.includes("javascript"))
  ) {
    return "xhr";
  }
  return "other";
}

export function classifyEntryResource(entry: HarEntrySummary): ResourceFilterId {
  if (entry.resource_type?.trim()) {
    return mapChromeResourceType(entry.resource_type);
  }
  return inferResourceType(entry);
}

export function matchesResourceFilter(
  entry: HarEntrySummary,
  filter: ResourceFilterId
): boolean {
  if (filter === "all") return true;
  return classifyEntryResource(entry) === filter;
}
