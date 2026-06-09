import { useCallback, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "@/lib/api";

function isHarPath(path: string): boolean {
  const lower = path.toLowerCase();
  return lower.endsWith(".har") || lower.endsWith(".json");
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
    let unlistenOpen: (() => void) | undefined;
    let unlistenDrop: (() => void) | undefined;

    void api.notifyFrontendReady().catch(console.error);

    api
      .takePendingHarFiles()
      .then((paths) => {
        if (paths[0]) {
          void openPath(paths[0]);
        }
      })
      .catch(console.error);

    listen<string[]>("open-har-files", (event) => {
      const path = event.payload[0];
      if (path) {
        void openPath(path);
      }
    }).then((fn) => {
      unlistenOpen = fn;
    });

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
      unlistenOpen?.();
      unlistenDrop?.();
      onDraggingChange?.(false);
    };
  }, [openPath, onDraggingChange]);
}
