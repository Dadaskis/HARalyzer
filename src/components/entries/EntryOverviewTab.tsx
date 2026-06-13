import { useMemo } from "react";
import type { HarEntryDetail } from "@/lib/types";
import { formatBytes, formatDuration, cn } from "@/lib/utils";
import {
  bodyByteLength,
  bodyPreview,
  collectNotableHeaders,
  parseEntryUrl,
  resourceTypeLabel,
  statusCategory,
} from "@/lib/entry-overview";

function OverviewSection({
  title,
  children,
  className,
}: {
  title: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("rounded-lg border border-primary/15 bg-[oklch(0.11_0.04_290)]", className)}>
      <h3 className="border-b border-primary/10 px-3 py-2 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
        {title}
      </h3>
      <div className="px-3 py-2">{children}</div>
    </section>
  );
}

function StatCard({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="rounded-md border border-primary/10 bg-[oklch(0.13_0.05_292)] px-2.5 py-2">
      <p className="text-[10px] text-muted-foreground">{label}</p>
      <p className="mt-0.5 font-mono text-sm font-medium text-foreground">{value}</p>
      {sub && <p className="mt-0.5 text-[10px] text-muted-foreground">{sub}</p>}
    </div>
  );
}

function DetailRow({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[minmax(0,110px)_1fr] gap-x-3 gap-y-0.5 border-b border-primary/5 py-1.5 last:border-0">
      <span className="text-[10px] text-muted-foreground">{label}</span>
      <span className="min-w-0 break-all font-mono text-[11px] text-foreground/90">{value}</span>
    </div>
  );
}

const toneClass = {
  success: "text-emerald-400",
  warning: "text-amber-400",
  error: "text-red-400",
  neutral: "text-muted-foreground",
} as const;

export function EntryOverviewTab({ entry }: { entry: HarEntryDetail }) {
  const s = entry.summary;
  const requestHeaders = entry.request_headers ?? [];
  const responseHeaders = entry.response_headers ?? [];

  const overview = useMemo(() => {
    const urlParts = parseEntryUrl(s.url);
    const status = statusCategory(s.status);
    const reqBodyLen = bodyByteLength(entry.request_body);
    const resBodyLen = bodyByteLength(entry.response_body);
    const notable = collectNotableHeaders(entry);
    const reqPreview = bodyPreview(entry.request_body);
    const resPreview = bodyPreview(entry.response_body);

    return {
      urlParts,
      status,
      reqBodyLen,
      resBodyLen,
      notable,
      reqPreview,
      resPreview,
      resource: resourceTypeLabel(s),
    };
  }, [entry, s, requestHeaders, responseHeaders]);

  return (
    <div className="space-y-3 pt-2 text-xs">
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        <StatCard label="Duration" value={formatDuration(s.time_ms)} />
        <StatCard
          label="Response size"
          value={formatBytes(s.size)}
          sub={overview.resBodyLen > 0 ? `${formatBytes(overview.resBodyLen)} body` : undefined}
        />
        <StatCard
          label="Request body"
          value={overview.reqBodyLen > 0 ? formatBytes(overview.reqBodyLen) : "—"}
        />
        <StatCard
          label="Headers"
          value={`${requestHeaders.length} / ${responseHeaders.length}`}
          sub="req / res"
        />
      </div>

      <OverviewSection title="General">
        <DetailRow
          label="Status"
          value={
            <span className={toneClass[overview.status.tone]}>
              {s.status} · {overview.status.label}
            </span>
          }
        />
        <DetailRow label="Resource" value={overview.resource} />
        {s.resource_type && <DetailRow label="HAR type" value={s.resource_type} />}
        <DetailRow label="Response MIME" value={s.mime_type || "—"} />
        {s.started_at && (
          <DetailRow label="Started" value={new Date(s.started_at).toLocaleString()} />
        )}
        <DetailRow label="Entry #" value={String(s.index)} />
      </OverviewSection>

      {overview.urlParts && (
        <OverviewSection title="URL">
          <DetailRow label="Host" value={overview.urlParts.host} />
          <DetailRow label="Path" value={overview.urlParts.pathname} />
          {overview.urlParts.query.length > 0 ? (
            <div className="mt-2 space-y-1">
              <p className="text-[10px] font-medium text-muted-foreground">Query parameters</p>
              {overview.urlParts.query.map(({ key, value }) => (
                <div
                  key={key}
                  className="break-all rounded bg-[oklch(0.13_0.05_292)] px-2 py-1 font-mono text-[10px]"
                >
                  <span className="text-primary/80">{key}</span>
                  {value !== "" && <> = {value}</>}
                </div>
              ))}
            </div>
          ) : (
            <DetailRow label="Query" value="—" />
          )}
        </OverviewSection>
      )}

      {overview.notable.length > 0 && (
        <OverviewSection title="Notable headers">
          <div className="space-y-1">
            {overview.notable.map((item) => (
              <div key={item.label} className="rounded bg-[oklch(0.13_0.05_292)] px-2 py-1.5">
                <p className="text-[10px] text-muted-foreground">{item.label}</p>
                <p className="mt-0.5 break-all font-mono text-[10px] text-foreground/90">
                  {item.value}
                </p>
              </div>
            ))}
          </div>
        </OverviewSection>
      )}

      {(overview.reqPreview || overview.resPreview) && (
        <OverviewSection title="Body preview">
          {overview.reqPreview && (
            <div className="mb-2">
              <p className="mb-1 text-[10px] font-medium text-muted-foreground">Request</p>
              <pre className="max-h-24 overflow-auto rounded bg-[oklch(0.13_0.05_292)] p-2 font-mono text-[10px] whitespace-pre-wrap break-all">
                {overview.reqPreview}
              </pre>
            </div>
          )}
          {overview.resPreview && (
            <div>
              <p className="mb-1 text-[10px] font-medium text-muted-foreground">Response</p>
              <pre className="max-h-24 overflow-auto rounded bg-[oklch(0.13_0.05_292)] p-2 font-mono text-[10px] whitespace-pre-wrap break-all">
                {overview.resPreview}
              </pre>
            </div>
          )}
        </OverviewSection>
      )}

      {entry.js_insights.length > 0 && (
        <OverviewSection title="Detected JS patterns">
          <ul className="space-y-1">
            {entry.js_insights.map((insight, i) => (
              <li
                key={i}
                className="rounded bg-[oklch(0.13_0.05_292)] px-2 py-1 font-mono text-[10px] whitespace-pre-wrap break-words"
              >
                {insight}
              </li>
            ))}
          </ul>
        </OverviewSection>
      )}
    </div>
  );
}
