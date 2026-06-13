import { useCallback, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "@/lib/api";

function isHarPath(path: string): boolean {
  const lower = path.trim().toLowerCase();
  return lower.endsWith(".har") || lower.endsWith(".json");
}

async function drainPendingHarFiles(
  openPath: (filePath: string) => Promise<void>
): Promise<void> {
  const paths = await api.takePendingHarFiles();
  if (paths[0]) {
    await openPath(paths[0]);
  }
}

export function useHarFileOpen(
  onOpenPath: (filePath: string) => void | Promise<void>,
  onDraggingChange?: (dragging: boolean) => void
) {
  const onOpenRef = useRef(onOpenPath);
  onOpenRef.current = onOpenPath;

  const openPath = useCallback(async (filePath: string) => {
    const normalized = filePath.trim();
    if (!normalized || !isHarPath(normalized)) return;
    await onOpenRef.current(normalized);
    await api.ackPendingHarFiles([normalized]).catch(console.error);
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlistenOpen: (() => void) | undefined;
    let unlistenDrop: (() => void) | undefined;
    let retryTimer: ReturnType<typeof setInterval> | undefined;

    const bootstrap = async () => {
      unlistenOpen = await listen<string[]>("open-har-files", (event) => {
        const path = event.payload[0];
        if (path) {
          void openPath(path);
        }
      });

      if (cancelled) return;

      await drainPendingHarFiles(openPath);
      await api.notifyFrontendReady().catch(console.error);

      if (cancelled) return;

      let retries = 0;
      retryTimer = setInterval(() => {
        retries += 1;
        if (retries > 10) {
          clearInterval(retryTimer);
          retryTimer = undefined;
          return;
        }
        void drainPendingHarFiles(openPath);
      }, 500);
    };

    void bootstrap();

    getCurrentWindow()
      .onDragDropEvent((event) => {
        if (event.payload.type === "enter" || event.payload.type === "over") {
          onDraggingChange?.(true);
        } else if (event.payload.type === "leave") {
          onDraggingChange?.(false);
        } else if (event.payload.type === "drop") {
          onDraggingChange?.(false);
          const harPath = event.payload.paths.find(isHarPath);
          if (harPath) {
            void openPath(harPath);
          }
        }
      })
      .then((fn) => {
        unlistenDrop = fn;
      })
      .catch(console.error);

    return () => {
      cancelled = true;
      unlistenOpen?.();
      unlistenDrop?.();
      if (retryTimer) clearInterval(retryTimer);
      onDraggingChange?.(false);
    };
  }, [openPath, onDraggingChange]);
}
