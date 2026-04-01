"use client";

import { useEffect, useRef, useState, useCallback } from "react";
import { ZoomIn, ZoomOut, RefreshCw, Loader2, FileText, AlertCircle } from "lucide-react";
import { useResumeStore } from "@/store/resumeStore";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";

import type { RenderTask } from "pdfjs-dist";

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

export function PdfPreview() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const { renderJobId, renderStatus, rerender, resumeId } = useResumeStore();
  const [isLoading, setIsLoading] = useState(false);
  const [renderError, setRenderError] = useState<string | null>(null);
  const [zoom, setZoom] = useState(ZOOM_DEFAULT);
  // Track rendered canvas dimensions so the wrapper div can reflect the scaled size,
  // giving overflow-auto a real layout size to scroll against.
  const [canvasDims, setCanvasDims] = useState({ width: 0, height: 0 });
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const activeRenderTaskRef = useRef<RenderTask | null>(null);

  // Renders the PDF once at RENDER_SCALE. Zoom is pure CSS — no re-render needed.
  const renderPdf = useCallback(async (jobId: string) => {
    if (!canvasRef.current) return;

    if (activeRenderTaskRef.current) {
      activeRenderTaskRef.current.cancel();
      activeRenderTaskRef.current = null;
    }

    setIsLoading(true);
    setRenderError(null);

    try {
      if (!pdfjsLib) {
        pdfjsLib = await import("pdfjs-dist");
        pdfjsLib.GlobalWorkerOptions.workerSrc = "/pdf.worker.min.mjs";
      }

      const pdfUrl = api.getPdfUrl(jobId);
      const loadingTask = pdfjsLib.getDocument(pdfUrl);
      const pdf = await loadingTask.promise;

      if (pdf.numPages === 0) {
        throw new Error("PDF contains no pages — try generating again.");
      }

      const page = await pdf.getPage(1);
      if (!page) {
        throw new Error("Failed to load page 1 — try generating again.");
      }

      const canvas = canvasRef.current;
      if (!canvas) return;

      const viewport = page.getViewport({ scale: RENDER_SCALE });
      canvas.height = viewport.height;
      canvas.width = viewport.width;
      setCanvasDims({ width: viewport.width, height: viewport.height });

      const task = page.render({ canvas, viewport });
      activeRenderTaskRef.current = task;
      await task.promise;
      activeRenderTaskRef.current = null;
    } catch (e) {
      if (e instanceof Error && e.name === "RenderingCancelledException") return;
      const msg = e instanceof Error ? e.message : "Unknown render error";
      setRenderError(msg);
      console.error("[PdfPreview] PDF render error:", e);
    } finally {
      setIsLoading(false);
    }
  }, []);

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

  // renderStatus === "done" — show toolbar + scrollable canvas
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
        </div>
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

      {/* Scrollable canvas area */}
      <div className="flex-1 overflow-auto relative">
        {isLoading && (
          <div className="absolute inset-0 flex items-center justify-center bg-background/50 z-10">
            <div className="h-8 w-8 animate-spin rounded-full border-2 border-current border-t-transparent text-muted-foreground" />
          </div>
        )}
        {/*
          The wrapper div is sized to the actual scaled dimensions so overflow-auto
          has real layout space to scroll against. The canvas is then CSS-scaled
          with transform-origin: top left so it fills that space exactly.
        */}
        <div className="p-2">
          <div style={{
            width: canvasDims.width * zoom,
            height: canvasDims.height * zoom,
          }}>
            <canvas
              ref={canvasRef}
              className="shadow-lg rounded"
              style={{ transform: `scale(${zoom})`, transformOrigin: "top left" }}
            />
          </div>
        </div>
      </div>
    </div>
  );
}
