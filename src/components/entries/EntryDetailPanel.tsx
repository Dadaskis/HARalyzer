import { useState } from "react";
import type { ReactNode } from "react";
import { X, MessageSquare, Code2, Copy, ChevronDown } from "lucide-react";
import { Panel, PanelGroup, PanelResizeHandle } from "react-resizable-panels";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { BodyViewer } from "@/components/entries/BodyViewer";
import { EntryJsViewer } from "@/components/entries/EntryJsViewer";
import { EntryOverviewTab } from "@/components/entries/EntryOverviewTab";
import type { HarEntryDetail } from "@/lib/types";
import {
  entryAsCurl,
  entryAsHarSource,
  entryHeadersText,
  entryUrlLine,
} from "@/lib/entry-copy";
import { formatBytes, formatDuration } from "@/lib/utils";

interface EntryDetailPanelProps {
  sessionId: string;
  entry: HarEntryDetail;
  onClose: () => void;
  onAskAi: (entry: HarEntryDetail) => void;
  onDeobfuscated?: (entryIndex: number) => void;
}

async function copyText(text: string) {
  await navigator.clipboard.writeText(text);
}

export function EntryDetailPanel({
  sessionId,
  entry,
  onClose,
  onAskAi,
  onDeobfuscated,
}: EntryDetailPanelProps) {
  const [copyOpen, setCopyOpen] = useState(false);
  const s = entry.summary;
  const requestHeaders = entry.request_headers ?? [];
  const responseHeaders = entry.response_headers ?? [];
  const requestMime =
    requestHeaders.find((h) => h.name.toLowerCase() === "content-type")?.value ??
    (entry.request_body.trim().startsWith("{") ? "application/json" : undefined);
  const responseMime = s.mime_type || undefined;

  const copyOptions = [
    { label: "URL line", text: entryUrlLine(entry) },
    { label: "cURL command", text: entryAsCurl(entry) },
    { label: "Request headers", text: entryHeadersText(entry, "request") },
    { label: "Response headers", text: entryHeadersText(entry, "response") },
    { label: "All headers", text: entryHeadersText(entry, "both") },
    { label: "Request body", text: entry.request_body },
    { label: "Response body", text: entry.response_body },
    { label: "Entry JSON", text: entryAsHarSource(entry) },
  ];

  return (
    <div className="flex h-full flex-col border-b border-primary/30 bg-[oklch(0.13_0.055_290)] shadow-[inset_0_-1px_0_oklch(0.55_0.2_295/0.2)]">
      <div className="flex shrink-0 items-center gap-2 border-b border-primary/20 px-4 py-2.5">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="outline" className="font-mono">
              {s.method}
            </Badge>
            <Badge
              variant={
                s.status >= 400
                  ? "destructive"
                  : s.status >= 200 && s.status < 300
                    ? "success"
                    : "secondary"
              }
            >
              {s.status}
            </Badge>
            {s.is_javascript && (
              <Badge variant="outline" className="gap-1">
                <Code2 className="h-3 w-3" />
                JS
              </Badge>
            )}
            <span className="text-[10px] text-muted-foreground">
              #{s.index} · {formatBytes(s.size)} · {formatDuration(s.time_ms)}
            </span>
          </div>
          <p className="mt-1 truncate font-mono text-xs text-foreground/90" title={s.url}>
            {s.url}
          </p>
        </div>
        <div className="relative">
          <Button
            size="sm"
            variant="outline"
            onClick={() => setCopyOpen((v) => !v)}
          >
            <Copy className="h-3.5 w-3.5" />
            Copy
            <ChevronDown className="h-3 w-3 opacity-60" />
          </Button>
          {copyOpen && (
            <>
              <div
                className="fixed inset-0 z-40"
                onClick={() => setCopyOpen(false)}
                role="presentation"
              />
              <div className="absolute right-0 top-full z-50 mt-1 min-w-[180px] rounded-md border border-border bg-popover py-1 shadow-lg">
                {copyOptions.map((opt) => (
                  <button
                    key={opt.label}
                    type="button"
                    className="block w-full px-3 py-1.5 text-left text-xs hover:bg-muted"
                    onClick={() => {
                      void copyText(opt.text);
                      setCopyOpen(false);
                    }}
                  >
                    {opt.label}
                  </button>
                ))}
              </div>
            </>
          )}
        </div>
        <Button size="sm" variant="outline" onClick={() => onAskAi(entry)}>
          <MessageSquare />
          Ask AI
        </Button>
        <Button size="icon" variant="ghost" onClick={onClose}>
          <X className="h-4 w-4" />
        </Button>
      </div>

      <Tabs defaultValue="overview" className="flex min-h-0 flex-1 flex-col">
        <TabsList className="mx-4 mt-2 w-auto shrink-0 justify-start bg-[oklch(0.16_0.06_290)]">
          <TabsTrigger value="overview">Overview</TabsTrigger>
          <TabsTrigger value="request">Request</TabsTrigger>
          <TabsTrigger value="response">Response</TabsTrigger>
          {s.is_javascript && <TabsTrigger value="js">JS</TabsTrigger>}
        </TabsList>

        <TabsContent value="overview" className="mt-0 min-h-0 flex-1 overflow-hidden">
          <ScrollArea className="h-full px-4 pb-4">
            <EntryOverviewTab entry={entry} />
          </ScrollArea>
        </TabsContent>

        <TabsContent value="request" className="mt-0 min-h-0 flex-1 overflow-hidden">
          <ScrollArea className="h-full px-4 pb-4">
            <div className="space-y-3 pt-2">
              <div>
                <p className="mb-1 text-xs font-medium text-muted-foreground">Headers</p>
                <div className="rounded-lg border border-primary/15 bg-[oklch(0.11_0.04_290)] p-2">
                  {requestHeaders.length === 0 ? (
                    <p className="text-[10px] text-muted-foreground">No headers</p>
                  ) : (
                    requestHeaders.map((h, i) => (
                      <div
                        key={`${h.name}-${i}`}
                        className="font-mono text-[10px] whitespace-pre-wrap break-words [overflow-wrap:anywhere]"
                      >
                        <span className="text-primary/80">{h.name}</span>: {h.value}
                      </div>
                    ))
                  )}
                </div>
              </div>
              <div>
                <p className="mb-1 text-xs font-medium text-muted-foreground">Body</p>
                <BodyViewer
                  body={entry.request_body}
                  mimeType={requestMime}
                  emptyLabel="No request body"
                  sessionId={sessionId}
                  entryIndex={s.index}
                  bodyField="request"
                />
              </div>
            </div>
          </ScrollArea>
        </TabsContent>

        <TabsContent value="response" className="mt-0 flex min-h-0 flex-1 flex-col overflow-hidden">
          {s.is_javascript ? (
            <div className="flex h-full min-h-0 flex-col gap-3 px-4 pb-4 pt-2">
              <div className="shrink-0">
                <p className="mb-1 text-xs font-medium text-muted-foreground">Headers</p>
                <div className="max-h-28 overflow-auto rounded-lg border border-primary/15 bg-[oklch(0.11_0.04_290)] p-2">
                  {responseHeaders.slice(0, 40).map((h, i) => (
                    <div
                      key={`${h.name}-${i}`}
                      className="font-mono text-[10px] whitespace-pre-wrap break-words [overflow-wrap:anywhere]"
                    >
                      <span className="text-primary/80">{h.name}</span>: {h.value}
                    </div>
                  ))}
                </div>
              </div>
              <div className="flex min-h-0 flex-1 flex-col">
                <p className="mb-1 shrink-0 text-xs font-medium text-muted-foreground">Body</p>
                <BodyViewer
                  fill
                  body={entry.response_body}
                  mimeType={responseMime}
                  emptyLabel="No response body"
                  isJavaScript
                  sessionId={sessionId}
                  entryIndex={s.index}
                  bodyField="response"
                />
              </div>
            </div>
          ) : (
            <ScrollArea className="h-full px-4 pb-4">
              <div className="space-y-3 pt-2">
                <div>
                  <p className="mb-1 text-xs font-medium text-muted-foreground">Headers</p>
                  <div className="rounded-lg border border-primary/15 bg-[oklch(0.11_0.04_290)] p-2">
                    {responseHeaders.slice(0, 40).map((h, i) => (
                      <div
                        key={`${h.name}-${i}`}
                        className="font-mono text-[10px] whitespace-pre-wrap break-words [overflow-wrap:anywhere]"
                      >
                        <span className="text-primary/80">{h.name}</span>: {h.value}
                      </div>
                    ))}
                  </div>
                </div>
                <div>
                  <p className="mb-1 text-xs font-medium text-muted-foreground">Body</p>
                  <BodyViewer
                    body={entry.response_body}
                    mimeType={responseMime}
                    emptyLabel="No response body"
                    sessionId={sessionId}
                    entryIndex={s.index}
                    bodyField="response"
                  />
                </div>
              </div>
            </ScrollArea>
          )}
        </TabsContent>

        {s.is_javascript && (
          <TabsContent value="js" className="mt-0 flex min-h-0 flex-1 flex-col overflow-hidden">
            <div className="h-full min-h-0 px-4 pb-4">
              <EntryJsViewer
                sessionId={sessionId}
                entryIndex={s.index}
                source={entry.response_body}
                deobfuscatedJs={entry.deobfuscated_js}
                onDeobfuscated={() => onDeobfuscated?.(s.index)}
              />
            </div>
          </TabsContent>
        )}
      </Tabs>
    </div>
  );
}

interface EntryDetailLayoutProps {
  sessionId: string;
  entry: HarEntryDetail | null;
  onClose: () => void;
  onAskAi: (entry: HarEntryDetail) => void;
  onDeobfuscated?: (entryIndex: number) => void;
  children: ReactNode;
}

export function EntryDetailLayout({
  sessionId,
  entry,
  onClose,
  onAskAi,
  onDeobfuscated,
  children,
}: EntryDetailLayoutProps) {
  if (!entry?.summary) {
    return <>{children}</>;
  }

  return (
    <PanelGroup direction="vertical" className="h-full">
      <Panel defaultSize={42} minSize={25} maxSize={65}>
        <EntryDetailPanel
          sessionId={sessionId}
          entry={entry}
          onClose={onClose}
          onAskAi={onAskAi}
          onDeobfuscated={onDeobfuscated}
        />
      </Panel>
      <PanelResizeHandle className="h-1 bg-primary/25 transition-colors hover:bg-primary/50" />
      <Panel defaultSize={58} minSize={35}>
        {children}
      </Panel>
    </PanelGroup>
  );
}
