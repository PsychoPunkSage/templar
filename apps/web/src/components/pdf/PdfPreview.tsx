"use client";

import { useEffect, useRef, useState, useCallback } from "react";
import { ZoomIn, ZoomOut, RefreshCw, Loader2, FileText, AlertCircle, Download } from "lucide-react";
import { useResumeStore } from "@/store/resumeStore";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";

type PdfjsLib = typeof import("pdfjs-dist");
let pdfjsLib: PdfjsLib | null = null;

// Canvas is rendered once at this fixed scale for crisp text.
// Zoom is applied via CSS transform — no re-render on zoom changes.
const RENDER_SCALE = 2.0;

const ZOOM_MIN = 0.5;
const ZOOM_MAX = 3.0;
const ZOOM_STEP = 0.25;
// Default zoom: show the canvas at its natural 1:1 pixel size
const ZOOM_DEFAULT = 1.0;

/** Per-page rasterized state. */
interface RenderedPage {
  width: number;
  height: number;
  /** ImageBitmap or canvas — we store the canvas element directly. */
  canvas: HTMLCanvasElement;
}

export function PdfPreview() {
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const { renderJobId, renderStatus, rerender, resumeId } = useResumeStore();
  const [isLoading, setIsLoading] = useState(false);
  const [renderError, setRenderError] = useState<string | null>(null);
  const [zoom, setZoom] = useState(ZOOM_DEFAULT);
  const [isDownloading, setIsDownloading] = useState(false);
  const [pages, setPages] = useState<RenderedPage[]>([]);
  const [totalPages, setTotalPages] = useState(0);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Monotonically increasing counter — each renderPdf call claims a generation.
  const renderGenRef = useRef(0);

  // Renders ALL pages of the PDF at RENDER_SCALE into offscreen canvases.
  // Stores them in state so React can mount them into the DOM.
  const renderPdf = useCallback(async (jobId: string) => {
    // Claim this generation — any older concurrent call will bail at the next checkpoint
    const myGen = ++renderGenRef.current;

    setIsLoading(true);
    setRenderError(null);
    setPages([]);

    try {
      if (!pdfjsLib) {
        pdfjsLib = await import("pdfjs-dist");
        pdfjsLib.GlobalWorkerOptions.workerSrc = "/pdf.worker.min.mjs";
      }

      if (myGen !== renderGenRef.current) return;

      const pdfUrl = api.getPdfUrl(jobId);
      const loadingTask = pdfjsLib.getDocument(pdfUrl);
      const pdf = await loadingTask.promise;

      if (myGen !== renderGenRef.current) return;

      const numPages = pdf.numPages;
      if (numPages === 0) {
        throw new Error("PDF contains no pages — try generating again.");
      }

      setTotalPages(numPages);

      // Render pages sequentially to avoid overwhelming the PDF.js worker
      const rendered: RenderedPage[] = [];
      for (let pageNum = 1; pageNum <= numPages; pageNum++) {
        if (myGen !== renderGenRef.current) return;

        const page = await pdf.getPage(pageNum);
        const viewport = page.getViewport({ scale: RENDER_SCALE });

        const canvas = document.createElement("canvas");
        canvas.width = viewport.width;
        canvas.height = viewport.height;

        const renderTask = page.render({ canvas, viewport });
        await renderTask.promise;

        rendered.push({ width: viewport.width, height: viewport.height, canvas });
        // Publish pages progressively so the user sees them appear one by one
        if (myGen === renderGenRef.current) {
          setPages([...rendered]);
        }
      }
    } catch (e) {
      if (myGen !== renderGenRef.current) return;
      if (e instanceof Error && e.name === "RenderingCancelledException") return;
      const msg = e instanceof Error ? e.message : "Unknown render error";
      setRenderError(msg);
      console.error("[PdfPreview] PDF render error:", e);
    } finally {
      if (myGen === renderGenRef.current) setIsLoading(false);
    }
  }, []);

  const handleDownload = useCallback(async () => {
    if (!renderJobId) return;
    setIsDownloading(true);
    try {
      await api.downloadPdf(renderJobId);
    } catch (e) {
      console.error("[PdfPreview] Download failed:", e);
    } finally {
      setIsDownloading(false);
    }
  }, [renderJobId]);

  // Re-render (fetch + rasterize) only when a new render job completes
  useEffect(() => {
    if (renderStatus !== "done" || !renderJobId) return;
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => renderPdf(renderJobId), 300);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [renderJobId, renderStatus, renderPdf]);

  // ─── Non-done render states ─────────────────────────────────────────────────

  if (renderStatus === "idle") {
    return (
      <div className="flex h-full items-center justify-center flex-col gap-3 text-muted-foreground">
        <FileText className="h-10 w-10 opacity-30" />
        <span className="text-sm">PDF preview will appear here after generation.</span>
      </div>
    );
  }

  if (renderStatus === "queued") {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="flex flex-col items-center gap-3 text-muted-foreground">
          <Loader2 className="h-8 w-8 animate-spin" />
          <span className="text-sm">Queued for rendering...</span>
        </div>
      </div>
    );
  }

  if (renderStatus === "rendering") {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="flex flex-col items-center gap-3 text-muted-foreground">
          <Loader2 className="h-8 w-8 animate-spin" />
          <span className="text-sm">Rendering PDF...</span>
        </div>
      </div>
    );
  }

  if (renderStatus === "failed" || renderError) {
    return (
      <div className="flex h-full items-center justify-center flex-col gap-3">
        <AlertCircle className="h-8 w-8 text-destructive opacity-80" />
        <span className="text-destructive text-sm font-medium">PDF render failed.</span>
        {renderError && (
          <span className="text-xs text-muted-foreground max-w-xs text-center">{renderError}</span>
        )}
        <Button
          size="sm"
          variant="outline"
          onClick={() => rerender()}
          disabled={!resumeId}
        >
          Retry Render
        </Button>
      </div>
    );
  }

  // renderStatus === "done" — show toolbar + scrollable multi-page canvas area
  return (
    <div className="flex flex-col h-full">
      {/* Toolbar */}
      <div className="flex items-center justify-between px-3 py-1.5 border-b bg-background shrink-0">
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={() => setZoom((z) => Math.max(ZOOM_MIN, parseFloat((z - ZOOM_STEP).toFixed(2))))}
            disabled={zoom <= ZOOM_MIN}
            aria-label="Zoom out"
          >
            <ZoomOut className="h-3.5 w-3.5" />
          </Button>
          <span className="text-xs text-muted-foreground w-10 text-center tabular-nums select-none">
            {Math.round(zoom * 100)}%
          </span>
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={() => setZoom((z) => Math.min(ZOOM_MAX, parseFloat((z + ZOOM_STEP).toFixed(2))))}
            disabled={zoom >= ZOOM_MAX}
            aria-label="Zoom in"
          >
            <ZoomIn className="h-3.5 w-3.5" />
          </Button>

          {/* Page count indicator — visible for multi-page documents */}
          {totalPages > 1 && (
            <span className="ml-2 text-xs text-muted-foreground">
              {pages.length}/{totalPages} pages
            </span>
          )}
        </div>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={handleDownload}
            disabled={isDownloading || !renderJobId}
            title="Download PDF"
            aria-label="Download PDF"
          >
            {isDownloading
              ? <Loader2 className="h-3.5 w-3.5 animate-spin" />
              : <Download className="h-3.5 w-3.5" />}
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={() => rerender()}
            disabled={!resumeId}
            title="Re-render PDF"
            aria-label="Re-render PDF"
          >
            <RefreshCw className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>

      {/* Scrollable multi-page canvas area */}
      <div ref={scrollContainerRef} className="flex-1 overflow-auto relative">
        {isLoading && pages.length === 0 && (
          <div className="absolute inset-0 flex items-center justify-center bg-background/50 z-10">
            <div className="h-8 w-8 animate-spin rounded-full border-2 border-current border-t-transparent text-muted-foreground" />
          </div>
        )}
        <div className="p-2 flex flex-col gap-3 items-start">
          {pages.map((pg, idx) => (
            <div
              key={idx}
              style={{
                width: pg.width * zoom,
                height: pg.height * zoom,
                position: "relative",
                flexShrink: 0,
              }}
            >
              {/* Mount the offscreen canvas into the DOM via ref callback */}
              <div
                style={{
                  transformOrigin: "top left",
                  transform: `scale(${zoom})`,
                  position: "absolute",
                  top: 0,
                  left: 0,
                }}
                ref={(node) => {
                  if (node && !node.contains(pg.canvas)) {
                    node.innerHTML = "";
                    node.appendChild(pg.canvas);
                  }
                }}
              />
              {/* Page number label for multi-page documents */}
              {totalPages > 1 && (
                <div
                  className="absolute -bottom-5 left-0 text-xs text-muted-foreground select-none"
                  style={{ width: pg.width * zoom }}
                >
                  Page {idx + 1}
                </div>
              )}
            </div>
          ))}
          {/* Loading indicator for progressive page loading */}
          {isLoading && pages.length > 0 && (
            <div className="flex items-center gap-2 text-xs text-muted-foreground py-2">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              Loading page {pages.length + 1} of {totalPages}...
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
