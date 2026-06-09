import { useEffect, useState } from "react";
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

interface SettingsDialogProps {
  open: boolean;
  onClose: () => void;
}

export function SettingsDialog({ open, onClose }: SettingsDialogProps) {
  const { settings, models, setSettings, setModels } = useSettingsStore();
  const [local, setLocal] = useState(settings);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open) setLocal(settings);
  }, [open, settings]);

  useEffect(() => {
    if (open) {
      api.listOpenRouterModels().then(setModels).catch(console.error);
    }
  }, [open, setModels]);

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

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
      <div className="w-full max-w-md rounded-xl border bg-card p-6 shadow-xl">
        <h2 className="mb-4 text-lg font-semibold">Settings</h2>
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
          <div className="space-y-2">
            <Label>Default Model</Label>
            <Select
              value={
                models.some((m) => m.id === local.default_model)
                  ? local.default_model
                  : undefined
              }
              onValueChange={(v) => setLocal({ ...local, default_model: v })}
            >
              <SelectTrigger>
                <SelectValue placeholder="Pick a model or enter ID below" />
              </SelectTrigger>
              <SelectContent>
                {models.map((m) => (
                  <SelectItem key={m.id} value={m.id}>
                    {m.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Input
              id="model-id"
              value={local.default_model}
              onChange={(e) =>
                setLocal({ ...local, default_model: e.target.value })
              }
              placeholder="Or enter any OpenRouter model ID (e.g. deepseek/deepseek-chat)"
              className="font-mono text-xs"
            />
            <p className="text-xs text-muted-foreground">
              Use the dropdown for common models, or type any model ID from{" "}
              <a
                href="https://openrouter.ai/models"
                target="_blank"
                rel="noreferrer"
                className="text-primary underline-offset-2 hover:underline"
              >
                openrouter.ai/models
              </a>
            </p>
          </div>
          <div className="space-y-2">
            <Label>Thinking Model (chat)</Label>
            <Select
              value={
                models.some((m) => m.id === local.thinking_model)
                  ? local.thinking_model
                  : undefined
              }
              onValueChange={(v) => setLocal({ ...local, thinking_model: v })}
            >
              <SelectTrigger>
                <SelectValue placeholder="Pick a reasoning model or enter ID below" />
              </SelectTrigger>
              <SelectContent>
                {models.map((m) => (
                  <SelectItem key={m.id} value={m.id}>
                    {m.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Input
              id="thinking-model-id"
              value={local.thinking_model}
              onChange={(e) =>
                setLocal({ ...local, thinking_model: e.target.value })
              }
              placeholder="e.g. deepseek/deepseek-r1 (used when Thinking mode is on in chat)"
              className="font-mono text-xs"
            />
            <p className="text-xs text-muted-foreground">
              Optional. When Thinking mode is enabled in chat, this model is used instead of the
              default model.
            </p>
          </div>
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
              Max LLM tool rounds per batch in chat before asking to continue (default: 10)
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
                  max_concurrent_requests: Math.min(16, Math.max(1, parseInt(e.target.value) || 4)),
                })
              }
            />
            <p className="text-xs text-muted-foreground">
              How many HAR chunks to analyze simultaneously (default: 4)
            </p>
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
        </div>
        <div className="mt-6 flex justify-end gap-2">
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
