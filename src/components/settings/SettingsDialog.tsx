import { useEffect, useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useSettingsStore } from "@/store/app-store";
import { api } from "@/lib/api";
import type {
  AgentLimitFieldDoc,
  AgentLimitsSettings,
  AppSettings,
  OpenRouterModel,
} from "@/lib/types";
import { DEFAULT_AGENT_LIMITS } from "@/lib/types";

interface SettingsDialogProps {
  open: boolean;
  onClose: () => void;
}

type SettingsTab = "general" | "models" | "limits";

function mergeSettings(raw: AppSettings): AppSettings {
  return {
    ...raw,
    smart_model_routing: raw.smart_model_routing ?? true,
    tier1_model: raw.tier1_model ?? "",
    tier2_model: raw.tier2_model ?? "",
    tier3_model: raw.tier3_model ?? "",
    provider: raw.provider ?? "",
    agent_limits: { ...DEFAULT_AGENT_LIMITS, ...(raw.agent_limits ?? {}) },
  };
}

function stripProvider(modelId: string): string {
  const pos = modelId.lastIndexOf(":");
  if (pos !== -1) {
    const base = modelId.slice(0, pos);
    if (base.includes("/")) return base;
  }
  return modelId;
}

function ModelPicker({
  label,
  hint,
  value,
  models,
  onChange,
  onInspect,
}: {
  label: string;
  hint: string;
  value: string;
  models: OpenRouterModel[];
  onChange: (v: string) => void;
  onInspect?: (id: string) => void;
}) {
  const resolvedValue = stripProvider(value);
  return (
    <div className="space-y-2">
      <Label>{label}</Label>
      <Select
        value={models.some((m) => m.id === resolvedValue) ? resolvedValue : undefined}
        onValueChange={(v) => {
          onChange(v);
          onInspect?.(v);
        }}
      >
        <SelectTrigger>
          <SelectValue placeholder="Pick a model or enter ID below" />
        </SelectTrigger>
        <SelectContent>
          {models.map((m) => (
            <SelectItem key={m.id} value={m.id}>
              {m.name}
              {m.context_length ? ` · ${Math.round(m.context_length / 1000)}K ctx` : ""}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onBlur={() => value.trim() && onInspect?.(value.trim())}
        placeholder="Or enter any OpenRouter model ID"
        className="font-mono text-xs"
      />
      <p className="text-xs text-muted-foreground">{hint}</p>
    </div>
  );
}

function ModelDetailCard({ model }: { model: OpenRouterModel | null }) {
  if (!model) {
    return (
      <div className="rounded-lg border border-dashed border-border p-4 text-sm text-muted-foreground">
        Select a model to see context window, pricing, tags, and capabilities inferred from
        OpenRouter metadata.
      </div>
    );
  }

  const caps = model.capabilities;
  const tags = caps?.tags ?? [];

  return (
    <div className="space-y-3 rounded-lg border border-border bg-muted/30 p-4 text-sm">
      <div>
        <p className="font-medium">{model.name}</p>
        <p className="font-mono text-xs text-muted-foreground">{model.id}</p>
      </div>
      {model.description && (
        <p className="text-xs text-muted-foreground line-clamp-4">{model.description}</p>
      )}
      <dl className="grid grid-cols-2 gap-x-3 gap-y-2 text-xs">
        <dt className="text-muted-foreground">Context</dt>
        <dd>{model.context_length ? `${model.context_length.toLocaleString()} tokens` : "—"}</dd>
        <dt className="text-muted-foreground">Budget tier</dt>
        <dd>{caps?.budget_tier ?? "—"}</dd>
        <dt className="text-muted-foreground">Prompt $/1M</dt>
        <dd>{model.pricing_prompt ?? "—"}</dd>
        <dt className="text-muted-foreground">Completion $/1M</dt>
        <dd>{model.pricing_completion ?? "—"}</dd>
        <dt className="text-muted-foreground">Modality</dt>
        <dd>{model.architecture_modality ?? "—"}</dd>
        <dt className="text-muted-foreground">Tokenizer</dt>
        <dd>{model.architecture_tokenizer ?? "—"}</dd>
      </dl>
      {tags.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {tags.map((t) => (
            <span
              key={t}
              className="rounded bg-primary/15 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-primary"
            >
              {t}
            </span>
          ))}
        </div>
      )}
      <div className="flex flex-wrap gap-2 text-xs">
        {caps?.code_focused && (
          <span className="rounded border border-border px-2 py-0.5">Code-focused</span>
        )}
        {caps?.reasoning_focused && (
          <span className="rounded border border-border px-2 py-0.5">Reasoning</span>
        )}
        {caps?.large_context && (
          <span className="rounded border border-border px-2 py-0.5">Large context</span>
        )}
      </div>
      {model.supported_parameters && model.supported_parameters.length > 0 && (
        <p className="text-[10px] text-muted-foreground">
          Params: {model.supported_parameters.slice(0, 12).join(", ")}
          {model.supported_parameters.length > 12 ? "…" : ""}
        </p>
      )}
    </div>
  );
}

function LimitField({
  doc,
  value,
  onChange,
}: {
  doc: AgentLimitFieldDoc;
  value: number;
  onChange: (v: number) => void;
}) {
  return (
    <div className="space-y-1">
      <Label htmlFor={`limit-${doc.key}`} className="text-sm">
        {doc.label}
      </Label>
      <Input
        id={`limit-${doc.key}`}
        type="number"
        min={0}
        value={value}
        onChange={(e) => onChange(parseInt(e.target.value, 10) || 0)}
        className="font-mono text-xs"
      />
      <p className="text-xs text-muted-foreground">{doc.description}</p>
    </div>
  );
}

export function SettingsDialog({ open, onClose }: SettingsDialogProps) {
  const { settings, models, setSettings, setModels } = useSettingsStore();
  const [local, setLocal] = useState(() => mergeSettings(settings));
  const [saving, setSaving] = useState(false);
  const [tab, setTab] = useState<SettingsTab>("general");
  const [limitDocs, setLimitDocs] = useState<AgentLimitFieldDoc[]>([]);
  const [inspectedModelId, setInspectedModelId] = useState<string | null>(null);

  useEffect(() => {
    if (open) setLocal(mergeSettings(settings));
  }, [open, settings]);

  useEffect(() => {
    if (open) {
      api.listOpenRouterModels().then(setModels).catch(console.error);
      api.getAgentLimitDocs().then(setLimitDocs).catch(console.error);
    }
  }, [open, setModels]);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [open, onClose]);

  const inspectedModel = useMemo(
    () => models.find((m) => m.id === stripProvider(inspectedModelId ?? "")) ?? null,
    [models, inspectedModelId]
  );

  const setLimit = (key: keyof AgentLimitsSettings, value: number) => {
    setLocal((s) => ({
      ...s,
      agent_limits: { ...s.agent_limits, [key]: value },
    }));
  };

  if (!open) return null;

  const handleSave = async () => {
    setSaving(true);
    try {
      await api.saveSettings(local);
      setSettings(local);
      onClose();
    } finally {
      setSaving(false);
    }
  };

  const tabClass = (t: SettingsTab) =>
    `rounded-md px-3 py-1.5 text-sm transition-colors ${
      tab === t ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:bg-muted"
    }`;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/85 p-4"
      onClick={onClose}
      role="presentation"
    >
      <div
        className="flex max-h-[min(90vh,820px)] w-full max-w-2xl flex-col overflow-hidden rounded-xl border border-border bg-card shadow-2xl"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-title"
      >
        <div className="flex shrink-0 items-center justify-between border-b border-border px-5 py-4">
          <h2 id="settings-title" className="text-lg font-semibold">
            Settings
          </h2>
          <Button type="button" variant="ghost" size="sm" onClick={onClose}>
            Close
          </Button>
        </div>

        <div className="flex shrink-0 gap-1 border-b border-border px-5 py-2">
          <button type="button" className={tabClass("general")} onClick={() => setTab("general")}>
            General
          </button>
          <button type="button" className={tabClass("models")} onClick={() => setTab("models")}>
            Models &amp; tiers
          </button>
          <button type="button" className={tabClass("limits")} onClick={() => setTab("limits")}>
            Agent limits
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {tab === "general" && (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="api-key">OpenRouter API Key</Label>
                <Input
                  id="api-key"
                  type="password"
                  placeholder="sk-or-..."
                  value={local.openrouter_api_key}
                  onChange={(e) =>
                    setLocal({ ...local, openrouter_api_key: e.target.value })
                  }
                />
              </div>
              <ModelPicker
                label="Default Model"
                hint="Fallback when tier routing is off or a tier slot is empty."
                value={local.default_model}
                models={models}
                onChange={(v) => setLocal({ ...local, default_model: v })}
                onInspect={setInspectedModelId}
              />
              <ModelPicker
                label="Thinking Model (chat)"
                hint="Used when Thinking mode is on in chat (overrides tier routing)."
                value={local.thinking_model}
                models={models}
                onChange={(v) => setLocal({ ...local, thinking_model: v })}
                onInspect={setInspectedModelId}
              />
              <div className="space-y-2">
                <Label htmlFor="chat-agent-steps">Chat agent tool steps</Label>
                <Input
                  id="chat-agent-steps"
                  type="number"
                  min={1}
                  max={50}
                  value={local.chat_agent_max_steps ?? 10}
                  onChange={(e) =>
                    setLocal({
                      ...local,
                      chat_agent_max_steps: Math.min(
                        50,
                        Math.max(1, parseInt(e.target.value) || 10)
                      ),
                    })
                  }
                />
                <p className="text-xs text-muted-foreground">
                  Max LLM tool rounds per batch before Continue (default: 10)
                </p>
              </div>
              <div className="space-y-2">
                <Label htmlFor="chunk-tokens">Chunk Max Tokens</Label>
                <Input
                  id="chunk-tokens"
                  type="number"
                  min={500}
                  max={100000}
                  value={local.chunk_max_tokens}
                  onChange={(e) =>
                    setLocal({
                      ...local,
                      chunk_max_tokens: parseInt(e.target.value) || 3000,
                    })
                  }
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="max-concurrent">Parallel LLM Requests</Label>
                <Input
                  id="max-concurrent"
                  type="number"
                  min={1}
                  max={16}
                  value={local.max_concurrent_requests}
                  onChange={(e) =>
                    setLocal({
                      ...local,
                      max_concurrent_requests: Math.min(
                        16,
                        Math.max(1, parseInt(e.target.value) || 4)
                      ),
                    })
                  }
                />
              </div>
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={local.filter_static_assets}
                  onChange={(e) =>
                    setLocal({ ...local, filter_static_assets: e.target.checked })
                  }
                  className="rounded border-input"
                />
                Filter static assets (images, fonts, CSS)
              </label>
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={local.analyze_javascript}
                  onChange={(e) =>
                    setLocal({ ...local, analyze_javascript: e.target.checked })
                  }
                  className="rounded border-input"
                />
                Analyze JavaScript (fetch/XHR/axios detection)
              </label>
              <div className="space-y-2">
                <Label htmlFor="agent-python-venv">Agent Python venv (optional)</Label>
                <Input
                  id="agent-python-venv"
                  value={local.agent_python_venv_path ?? ""}
                  onChange={(e) =>
                    setLocal({ ...local, agent_python_venv_path: e.target.value })
                  }
                  placeholder="/path/to/venv or C:\path\to\venv"
                  className="font-mono text-xs"
                />
              </div>
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={local.agent_allow_code_execution ?? true}
                  onChange={(e) =>
                    setLocal({ ...local, agent_allow_code_execution: e.target.checked })
                  }
                  className="rounded border-input"
                />
                Allow agent to run Python scripts
              </label>
            </div>
          )}

          {tab === "models" && (
            <div className="space-y-4">
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={local.smart_model_routing}
                  onChange={(e) =>
                    setLocal({ ...local, smart_model_routing: e.target.checked })
                  }
                  className="rounded border-input"
                />
                Smart 3-tier model routing
              </label>
              <p className="text-xs text-muted-foreground">
                The agent picks tier 1 (cheap) for early HAR discovery, tier 2 (balanced) for most
                work, and tier 3 (advanced) for scripts, failures, long context, or late-step
                escalation. Empty tier slots fall back to Default / Thinking models.
              </p>

              <ModelPicker
                label="Tier 1 — Fast / discovery"
                hint="Light tasks: list_entries, first-pass browsing. Prefer mini/flash models."
                value={local.tier1_model}
                models={models}
                onChange={(v) => setLocal({ ...local, tier1_model: v })}
                onInspect={setInspectedModelId}
              />
              <ModelPicker
                label="Tier 2 — Balanced"
                hint="Default agent work: get_entry, HTTP replay, most tool chains."
                value={local.tier2_model}
                models={models}
                onChange={(v) => setLocal({ ...local, tier2_model: v })}
                onInspect={setInspectedModelId}
              />
              <ModelPicker
                label="Tier 3 — Advanced"
                hint="Python scripts, stub recovery, step 6+ escalation, large-context payloads."
                value={local.tier3_model}
                models={models}
                onChange={(v) => setLocal({ ...local, tier3_model: v })}
                onInspect={setInspectedModelId}
              />

              <div className="space-y-2">
                <Label htmlFor="provider">OpenRouter provider override</Label>
                <Input
                  id="provider"
                  value={local.provider ?? ""}
                  onChange={(e) => setLocal({ ...local, provider: e.target.value })}
                  placeholder="e.g. deepseek, openai, anthropic"
                  className="font-mono text-xs"
                />
                <p className="text-xs text-muted-foreground">
                  Appends <code>:provider</code> to model IDs sent to OpenRouter. Use to force
                  a specific provider when the model is available on multiple providers.
                </p>
              </div>

              <div className="space-y-2">
                <Label>Model details</Label>
                <ModelDetailCard model={inspectedModel} />
              </div>
            </div>
          )}

          {tab === "limits" && (
            <div className="space-y-4">
              <p className="text-xs text-muted-foreground">
                Tune caps that were previously hardcoded. Output limits with override 0 scale
                automatically from the active model&apos;s context window.
              </p>
              {limitDocs.map((doc) => (
                <LimitField
                  key={doc.key}
                  doc={doc}
                  value={
                    local.agent_limits[doc.key as keyof AgentLimitsSettings] as number
                  }
                  onChange={(v) =>
                    setLimit(doc.key as keyof AgentLimitsSettings, v)
                  }
                />
              ))}
              {limitDocs.length === 0 && (
                <p className="text-sm text-muted-foreground">Loading limit descriptions…</p>
              )}
            </div>
          )}
        </div>

        <div className="flex shrink-0 justify-end gap-2 border-t border-border bg-card px-5 py-4">
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={handleSave} disabled={saving}>
            {saving ? "Saving..." : "Save"}
          </Button>
        </div>
      </div>
    </div>
  );
}
