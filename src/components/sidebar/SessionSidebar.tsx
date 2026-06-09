import { Trash2, FileJson, Clock } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import type { AnalysisSession } from "@/lib/types";
import { formatBytes } from "@/lib/utils";
import { cn } from "@/lib/utils";

interface SessionSidebarProps {
  sessions: AnalysisSession[];
  activeSessionId: string | null;
  onSelect: (id: string) => void;
  onDelete: (id: string) => void;
}

function statusColor(status: string) {
  switch (status) {
    case "complete":
      return "bg-emerald-500";
    case "analyzing":
      return "bg-blue-500 animate-pulse";
    case "parsed":
      return "bg-amber-500";
    default:
      return "bg-muted-foreground";
  }
}

export function SessionSidebar({
  sessions,
  activeSessionId,
  onSelect,
  onDelete,
}: SessionSidebarProps) {
  return (
    <div className="flex h-full flex-col border-r border-primary/20 bg-card/35 bloom-panel">
      <div className="border-b px-4 py-3">
        <h2 className="text-sm font-semibold">Recent Analyses</h2>
        <p className="text-xs text-muted-foreground">{sessions.length} sessions</p>
      </div>
      <ScrollArea className="flex-1">
        <div className="space-y-1 p-2">
          {sessions.length === 0 ? (
            <p className="px-2 py-4 text-center text-xs text-muted-foreground">
              No sessions yet
            </p>
          ) : (
            sessions.map((session) => (
              <div
                key={session.id}
                role="button"
                tabIndex={0}
                onClick={() => onSelect(session.id)}
                onKeyDown={(e) => e.key === "Enter" && onSelect(session.id)}
                className={cn(
                  "group flex w-full cursor-pointer flex-col gap-1 rounded-lg px-3 py-2 text-left transition-colors hover:bg-accent",
                  activeSessionId === session.id && "bg-accent"
                )}
              >
                <div className="flex items-start gap-2">
                  <FileJson className="mt-0.5 h-3.5 w-3.5 shrink-0 text-primary" />
                  <span className="min-w-0 flex-1 break-all text-xs font-medium leading-snug">
                    {session.file_name}
                  </span>
                  <div
                    className={cn(
                      "mt-1 h-2 w-2 shrink-0 rounded-full",
                      statusColor(session.status)
                    )}
                    title={session.status}
                  />
                </div>
                <div className="flex items-center gap-2 pl-5 text-[10px] text-muted-foreground">
                  <span>{session.total_entries.toLocaleString()} entries</span>
                  <span>·</span>
                  <span>{formatBytes(session.total_bytes)}</span>
                </div>
                <div className="flex items-center gap-1 pl-5 text-[10px] text-muted-foreground">
                  <Clock className="h-2.5 w-2.5" />
                  <span>{new Date(session.created_at).toLocaleString()}</span>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="ml-auto h-5 w-5 opacity-0 group-hover:opacity-100"
                    onClick={(e) => {
                      e.stopPropagation();
                      onDelete(session.id);
                    }}
                  >
                    <Trash2 className="h-3 w-3" />
                  </Button>
                </div>
              </div>
            ))
          )}
        </div>
      </ScrollArea>
    </div>
  );
}
