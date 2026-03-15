"use client";

import { useEffect, useRef, useState, useCallback } from "react";

// PDF.js is lazy-loaded client-side only to avoid SSR issues.
// The global is cached after first load so all thumbnail cards share one import.
type PdfjsLib = typeof import("pdfjs-dist");
let pdfjsLib: PdfjsLib | null = null;

interface TemplateThumbnailPdfProps {
  /** Full URL to the thumbnail-pdf endpoint for this template. */
  pdfUrl: string;
}

/**
 * Renders the first page of a template thumbnail PDF inside a card.
 *
 * Used in the "Choose a Template" picker grid. Replaces the broken <img> approach
 * that relied on S3 thumbnails generated asynchronously (often unavailable in dev).
 *
 * Design notes:
 * - Scale adapts to container width — fits any card size without fixed px values.
 * - PDF.js lazy-loaded once; subsequent cards share the same cached import.
 * - Canvas always in DOM (hidden, never removed) so canvasRef.current is stable.
 * - Error state is silent (no text) to keep cards visually clean.
 */
export function TemplateThumbnailPdf({ pdfUrl }: TemplateThumbnailPdfProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [hasError, setHasError] = useState(false);

  const renderPdf = useCallback(async (url: string) => {
    if (!url) {
      setIsLoading(false);
      setHasError(true);
      return;
    }

    setIsLoading(true);
    setHasError(false);

    try {
      // Lazy-load PDF.js only in the browser — avoids SSR issues
      if (!pdfjsLib) {
        pdfjsLib = await import("pdfjs-dist");
        pdfjsLib.GlobalWorkerOptions.workerSrc = "/pdf.worker.min.mjs";
      }

      const pdf = await pdfjsLib.getDocument(url).promise;
      const page = await pdf.getPage(1);

      const canvas = canvasRef.current;
      if (!canvas) throw new Error("Canvas ref not available");

      // Scale to fit the card's container width — re-measured at render time
      const containerWidth = canvas.parentElement?.clientWidth ?? 240;
      const unscaledVp = page.getViewport({ scale: 1 });
      const scale = containerWidth / unscaledVp.width;
      const viewport = page.getViewport({ scale });

      canvas.height = viewport.height;
      canvas.width = viewport.width;

      // PDF.js v5+: `canvas` parameter is required (replaces the legacy `canvasContext`)
      await page.render({ canvas, viewport }).promise;
    } catch {
      setHasError(true);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    renderPdf(pdfUrl);
  }, [pdfUrl, renderPdf]);

  return (
    <div className="w-full h-full relative">
      {/* Loading spinner */}
      {isLoading && (
        <div className="absolute inset-0 flex items-center justify-center bg-muted">
          <div className="h-5 w-5 animate-spin rounded-full border-2 border-muted-foreground border-t-transparent" />
        </div>
      )}

      {/* Error placeholder — minimal, keeps card layout intact */}
      {!isLoading && hasError && (
        <div className="absolute inset-0 flex items-center justify-center bg-muted text-muted-foreground text-xs">
          Preview unavailable
        </div>
      )}

      {/* Canvas — direct child of the always-visible outer div.
          `invisible` = visibility:hidden → stays in layout → canvas.parentElement.clientWidth is correct.
          `hidden` = display:none → removed from layout → clientWidth = 0 (the old bug). */}
      <canvas
        ref={canvasRef}
        className={`w-full ${isLoading || hasError ? "invisible" : ""}`}
      />
    </div>
  );
}
