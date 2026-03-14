"use client";

import { useEffect, useRef, useState, useCallback } from "react";

// PDF.js is lazy-loaded client-side only to avoid SSR issues.
// The global is cached after first load to avoid redundant dynamic imports
// (same pattern as PdfPreview.tsx).
type PdfjsLib = typeof import("pdfjs-dist");
let pdfjsLib: PdfjsLib | null = null;

interface StaticPdfPreviewProps {
  /** Full URL to a PDF endpoint. Fetched once on mount; re-fetched on url change. */
  pdfUrl: string;
}

/**
 * Renders the first page of a PDF at the given URL using PDF.js.
 *
 * Used in the editor right pane before resume generation — shows a compiled
 * preview of the selected template so the user knows what the output will look like.
 *
 * Design notes:
 * - Uses the same PDF.js lazy-load pattern as PdfPreview.tsx (shared worker setup).
 * - No debounce needed here: `pdfUrl` is stable (template id does not change on
 *   every keystroke). PDF is re-fetched only when `pdfUrl` prop changes.
 * - Scale is fixed at 1.2 — matches the right-pane width for A4/letter documents.
 */
export function StaticPdfPreview({ pdfUrl }: StaticPdfPreviewProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [renderError, setRenderError] = useState<string | null>(null);
  const slowTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [slowLoadWarning, setSlowLoadWarning] = useState(false);

  const renderPdf = useCallback(async (url: string) => {
    // Guard: never call pdfjsLib.getDocument("") — it hangs silently with no
    // error thrown and no timeout. Bail immediately with an error state instead.
    if (!url) {
      setIsLoading(false);
      setRenderError("No template URL provided");
      console.error("[StaticPdfPreview] pdfUrl is empty — aborting");
      return;
    }

    setIsLoading(true);
    setRenderError(null);
    setSlowLoadWarning(false);
    if (slowTimerRef.current) clearTimeout(slowTimerRef.current);
    slowTimerRef.current = setTimeout(() => setSlowLoadWarning(true), 15_000);

    console.log("[StaticPdfPreview] Fetching PDF from:", url);

    try {
      // Lazy-load PDF.js only in the browser — avoids SSR issues
      if (!pdfjsLib) {
        pdfjsLib = await import("pdfjs-dist");
        // Worker must be served as a static file from public/ (same as PdfPreview)
        pdfjsLib.GlobalWorkerOptions.workerSrc = "/pdf.worker.min.mjs";
      }

      console.log("[StaticPdfPreview] PDF.js loaded, requesting document...");
      const loadingTask = pdfjsLib.getDocument(url);
      const pdf = await loadingTask.promise;
      console.log("[StaticPdfPreview] PDF loaded, numPages:", pdf.numPages);
      const page = await pdf.getPage(1);

      const canvas = canvasRef.current;
      if (!canvas) throw new Error("Canvas ref not available");
      // Scale 1.2 gives a good fit inside the 50%-width right pane
      const viewport = page.getViewport({ scale: 1.2 });
      canvas.height = viewport.height;
      canvas.width = viewport.width;

      // PDF.js v5+: `canvas` parameter is required (replaces `canvasContext`)
      await page.render({ canvas, viewport }).promise;
      console.log("[StaticPdfPreview] Page rendered successfully");
    } catch (e) {
      const msg = e instanceof Error ? e.message : "Unknown render error";
      setRenderError(msg);
      console.error("[StaticPdfPreview] PDF load failed:", e);
    } finally {
      if (slowTimerRef.current) {
        clearTimeout(slowTimerRef.current);
        slowTimerRef.current = null;
      }
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    renderPdf(pdfUrl);
  }, [pdfUrl, renderPdf]);

  return (
    <div className="h-full">
      {/* Loading spinner — visible while isLoading */}
      {isLoading && (
        <div className="flex h-full items-center justify-center">
          <div className="flex flex-col items-center gap-3 text-muted-foreground">
            <div className="h-8 w-8 animate-spin rounded-full border-2 border-current border-t-transparent" />
            <span className="text-sm">
              {slowLoadWarning
                ? "Still compiling — first-time preview takes ~60s…"
                : "Loading template preview…"}
            </span>
          </div>
        </div>
      )}

      {/* Error state */}
      {!isLoading && renderError && (
        <div className="flex h-full items-center justify-center flex-col gap-2">
          <span className="text-muted-foreground text-sm font-medium">
            Template preview unavailable
          </span>
          <span className="text-xs text-muted-foreground max-w-xs text-center">
            The template PDF could not be rendered. This may be a temporary issue.
          </span>
        </div>
      )}

      {/* Canvas — ALWAYS in DOM so canvasRef.current is never null */}
      <div
        className={`flex flex-col items-center h-full overflow-auto p-4 gap-3 ${
          isLoading || renderError ? "hidden" : ""
        }`}
      >
        <canvas ref={canvasRef} className="shadow-lg rounded max-w-full" />
        <p className="text-xs text-muted-foreground text-center shrink-0">
          Template preview — generate your resume to see the PDF with your content.
        </p>
      </div>
    </div>
  );
}
