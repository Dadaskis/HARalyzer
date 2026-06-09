import type { ReactNode } from "react";
import { X, MessageSquare, Code2 } from "lucide-react";
import { Panel, PanelGroup, PanelResizeHandle } from "react-resizable-panels";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { BodyViewer, PlainBodyViewer } from "@/components/entries/BodyViewer";
import type { HarEntryDetail } from "@/lib/types";
import { formatBytes, formatDuration } from "@/lib/utils";

interface EntryDetailPanelProps {
  entry: HarEntryDetail;
  onClose: () => void;
  onAskAi: (entry: HarEntryDetail) => void;
}

export function EntryDetailPanel({ entry, onClose, onAskAi }: EntryDetailPanelProps) {
  const s = entry.summary;
  const requestHeaders = entry.request_headers ?? [];
  const responseHeaders = entry.response_headers ?? [];
  const jsInsights = entry.js_insights ?? [];
  const requestMime =
    requestHeaders.find((h) => h.name.toLowerCase() === "content-type")?.value ??
    (entry.request_body.trim().startsWith("{") ? "application/json" : undefined);
  const responseMime = s.mime_type || undefined;

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
            <div className="space-y-3 pt-2 text-xs">
              <div>
                <p className="mb-1 font-medium text-muted-foreground">MIME type</p>
                <p>{s.mime_type || "—"}</p>
              </div>
              {s.started_at && (
                <div>
                  <p className="mb-1 font-medium text-muted-foreground">Started</p>
                  <p>{new Date(s.started_at).toLocaleString()}</p>
                </div>
              )}
              {jsInsights.length > 0 && (
                <div>
                  <p className="mb-1 font-medium text-muted-foreground">Detected patterns</p>
                  <ul className="space-y-1">
                    {jsInsights.map((insight, i) => (
                      <li
                        key={i}
                        className="rounded bg-[oklch(0.16_0.05_290)] px-2 py-1 font-mono text-[10px] whitespace-pre-wrap break-words"
                      >
                        {insight}
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </div>
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
                />
              </div>
            </div>
          </ScrollArea>
        </TabsContent>

        <TabsContent value="response" className="mt-0 min-h-0 flex-1 overflow-hidden">
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
                />
              </div>
            </div>
          </ScrollArea>
        </TabsContent>

        {s.is_javascript && (
          <TabsContent value="js" className="mt-0 min-h-0 flex-1 overflow-hidden">
            <ScrollArea className="h-full px-4 pb-4">
              <PlainBodyViewer body={entry.response_body} emptyLabel="No source available" />
            </ScrollArea>
          </TabsContent>
        )}
      </Tabs>
    </div>
  );
}

interface EntryDetailLayoutProps {
  entry: HarEntryDetail | null;
  onClose: () => void;
  onAskAi: (entry: HarEntryDetail) => void;
  children: ReactNode;
}

export function EntryDetailLayout({
  entry,
  onClose,
  onAskAi,
  children,
}: EntryDetailLayoutProps) {
  if (!entry?.summary) {
    return <>{children}</>;
  }

  return (
    <PanelGroup direction="vertical" className="h-full">
      <Panel defaultSize={42} minSize={25} maxSize={65}>
        <EntryDetailPanel entry={entry} onClose={onClose} onAskAi={onAskAi} />
      </Panel>
      <PanelResizeHandle className="h-1 bg-primary/25 transition-colors hover:bg-primary/50" />
      <Panel defaultSize={58} minSize={35}>
        {children}
      </Panel>
    </PanelGroup>
  );
}
