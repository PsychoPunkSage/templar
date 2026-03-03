"use client";

import { useEffect, useRef, useState, useCallback } from "react";
import { useResumeStore } from "@/store/resumeStore";
import { api } from "@/lib/api";

// PDF.js is lazy-loaded client-side only to avoid SSR issues.
// The global is cached after first load to avoid redundant dynamic imports.
type PdfjsLib = typeof import("pdfjs-dist");
let pdfjsLib: PdfjsLib | null = null;

/**
 * In-browser PDF preview powered by PDF.js.
 *
 * Key constraints (from CLAUDE.md):
 * - PDF.js must only be loaded client-side (dynamic import inside useEffect)
 * - Preview updates are debounced at a minimum of 300ms
 * - Never re-renders on every keystroke — only when renderStatus transitions to 'done'
 */
export function PdfPreview() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { renderJobId, renderStatus } = useResumeStore();
  const [isLoading, setIsLoading] = useState(false);
  const [renderError, setRenderError] = useState<string | null>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const renderPdf = useCallback(async (jobId: string) => {
    if (!canvasRef.current) return;

    setIsLoading(true);
    setRenderError(null);

    try {
      // Lazy-load PDF.js only in the browser (avoids SSR issues)
      if (!pdfjsLib) {
        pdfjsLib = await import("pdfjs-dist");
        // Worker must be served as a static file from public/
        pdfjsLib.GlobalWorkerOptions.workerSrc = "/pdf.worker.min.mjs";
      }

      const pdfUrl = api.getPdfUrl(jobId);
      const loadingTask = pdfjsLib.getDocument(pdfUrl);
      const pdf = await loadingTask.promise;
      const page = await pdf.getPage(1);

      const canvas = canvasRef.current;
      const viewport = page.getViewport({ scale: 1.5 });
      canvas.height = viewport.height;
      canvas.width = viewport.width;

      // PDF.js v5+: `canvas` parameter is required (replaces `canvasContext`)
      await page.render({ canvas, viewport }).promise;
    } catch (e) {
      const msg = e instanceof Error ? e.message : "Unknown render error";
      setRenderError(msg);
      console.error("PDF render error:", e);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    if (renderStatus !== "done" || !renderJobId) return;

    // Debounce: enforce 300ms minimum between re-renders
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => renderPdf(renderJobId), 300);

    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [renderJobId, renderStatus, renderPdf]);

  // ─── Render states ─────────────────────────────────────────────────────────

  if (renderStatus === "idle") {
    return (
      <div className="flex h-full items-center justify-center text-muted-foreground text-sm">
        PDF preview will appear here after generation.
      </div>
    );
  }

  if (renderStatus === "queued") {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="flex flex-col items-center gap-3 text-muted-foreground">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-current border-t-transparent" />
          <span className="text-sm">Queued for rendering...</span>
        </div>
      </div>
    );
  }

  if (renderStatus === "rendering" || (isLoading && renderStatus === "done")) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="flex flex-col items-center gap-3 text-muted-foreground">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-current border-t-transparent" />
          <span className="text-sm">Rendering PDF...</span>
        </div>
      </div>
    );
  }

  if (renderStatus === "failed" || renderError) {
    return (
      <div className="flex h-full items-center justify-center flex-col gap-2">
        <span className="text-destructive text-sm font-medium">
          PDF render failed.
        </span>
        {renderError && (
          <span className="text-xs text-muted-foreground max-w-xs text-center">
            {renderError}
          </span>
        )}
        <span className="text-xs text-muted-foreground">
          Try generating again.
        </span>
      </div>
    );
  }

  // renderStatus === 'done' and not loading — show canvas
  return (
    <div className="flex justify-center overflow-auto h-full p-2">
      <canvas ref={canvasRef} className="shadow-lg rounded" />
      {isLoading && (
        <div className="absolute inset-0 flex items-center justify-center bg-background/50">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-current border-t-transparent text-muted-foreground" />
        </div>
      )}
    </div>
  );
}
