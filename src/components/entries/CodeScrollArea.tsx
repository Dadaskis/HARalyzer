import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { cn } from "@/lib/utils";

interface CodeScrollAreaProps {
  children: ReactNode;
  className?: string;
  viewportClassName?: string;
  fill?: boolean;
  forceWrap?: boolean;
}

export function CodeScrollArea({
  children,
  className,
  viewportClassName,
  fill = false,
  forceWrap = false,
}: CodeScrollAreaProps) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const hTrackRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const syncingRef = useRef(false);
  const [contentWidth, setContentWidth] = useState(0);
  const [viewportWidth, setViewportWidth] = useState(0);

  const needsHorizontal = !forceWrap && contentWidth > viewportWidth + 1;

  const measure = useCallback(() => {
    const viewport = viewportRef.current;
    const content = contentRef.current;
    if (!viewport || !content) return;
    setContentWidth(content.scrollWidth);
    setViewportWidth(viewport.clientWidth);
  }, []);

  useEffect(() => {
    measure();
    const viewport = viewportRef.current;
    const content = contentRef.current;
    if (!viewport || !content) return;

    const observer = new ResizeObserver(measure);
    observer.observe(viewport);
    observer.observe(content);
    return () => observer.disconnect();
  }, [measure, children]);

  const syncFromViewport = useCallback(() => {
    if (syncingRef.current) return;
    const viewport = viewportRef.current;
    const hTrack = hTrackRef.current;
    if (!viewport || !hTrack) return;
    syncingRef.current = true;
    hTrack.scrollLeft = viewport.scrollLeft;
    syncingRef.current = false;
  }, []);

  const syncFromTrack = useCallback(() => {
    if (syncingRef.current) return;
    const viewport = viewportRef.current;
    const hTrack = hTrackRef.current;
    if (!viewport || !hTrack) return;
    syncingRef.current = true;
    viewport.scrollLeft = hTrack.scrollLeft;
    syncingRef.current = false;
  }, []);

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;

    const onWheel = (event: WheelEvent) => {
      const horizontal = event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY);
      if (!horizontal) return;
      event.preventDefault();
      viewport.scrollLeft += event.deltaX !== 0 ? event.deltaX : event.deltaY;
      syncFromViewport();
    };

    viewport.addEventListener("wheel", onWheel, { passive: false });
    return () => viewport.removeEventListener("wheel", onWheel);
  }, [syncFromViewport]);

  return (
    <div className={cn("flex min-h-0 flex-col", fill && "flex-1", className)}>
      <div
        ref={viewportRef}
        onScroll={syncFromViewport}
        className={cn(
          "code-scroll-viewport min-h-0 overflow-y-auto overflow-x-hidden",
          fill ? "flex-1" : "max-h-56",
          viewportClassName
        )}
      >
        <div ref={contentRef} className="w-max min-w-full">
          {children}
        </div>
      </div>
      {needsHorizontal && (
        <div
          ref={hTrackRef}
          onScroll={syncFromTrack}
          className="code-scroll-htrack shrink-0 overflow-x-auto overflow-y-hidden border-t border-primary/15 bg-[oklch(0.13_0.05_292)]"
          aria-hidden
        >
          <div style={{ width: contentWidth, height: 1 }} />
        </div>
      )}
    </div>
  );
}
