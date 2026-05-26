"use client";

import { useEffect, useRef, useState, useCallback } from "react";
import { useRouter } from "next/navigation";
import { motion, type Variants } from "framer-motion";
import { Clock, Trash2, ArrowRight } from "lucide-react";
import type { CvProject } from "@templar/types";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";

// ── PDF.js singleton (shared module-level cache) ─────────────────────────────
type PdfjsLib = typeof import("pdfjs-dist");
let pdfjsLib: PdfjsLib | null = null;

// ── Helpers ───────────────────────────────────────────────────────────────────

function formatDate(iso: string) {
  return new Date(iso).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

// ── CvPlaceholder — pure CSS wireframe shown when no PDF is ready ─────────────

function CvPlaceholder({ name, templateId }: { name: string; templateId: string }) {
  const initial = name.charAt(0).toUpperCase();

  return (
    <div className="w-full h-full flex flex-col bg-muted/20 p-4 gap-2 select-none">
      {/* Header row */}
      <div className="flex items-center gap-2 pb-2 border-b border-border/40">
        <div className="w-5 h-5 rounded-full bg-primary/30 flex items-center justify-center shrink-0">
          <span className="text-[9px] font-mono font-bold text-primary">{initial}</span>
        </div>
        <div className="flex flex-col gap-1 flex-1">
          <div className="h-2 bg-foreground/20 rounded-full w-24" />
          <div className="h-1.5 bg-foreground/10 rounded-full w-16" />
        </div>
        <div className="h-1.5 bg-foreground/10 rounded-full w-12" />
      </div>

      {/* Section 1 */}
      <div className="flex flex-col gap-1.5 pt-1">
        <div className="h-1.5 bg-primary/20 rounded-full w-14" />
        <div className="flex flex-col gap-1">
          <div className="h-1.5 bg-foreground/10 rounded-full w-full" />
          <div className="h-1.5 bg-foreground/10 rounded-full w-5/6" />
          <div className="h-1.5 bg-foreground/10 rounded-full w-4/6" />
        </div>
      </div>

      {/* Section 2 */}
      <div className="flex flex-col gap-1.5 pt-1">
        <div className="h-1.5 bg-primary/20 rounded-full w-20" />
        <div className="flex flex-col gap-1">
          <div className="h-1.5 bg-foreground/10 rounded-full w-full" />
          <div className="h-1.5 bg-foreground/10 rounded-full w-3/4" />
          <div className="h-1.5 bg-foreground/10 rounded-full w-5/6" />
          <div className="h-1.5 bg-foreground/10 rounded-full w-2/3" />
        </div>
      </div>

      {/* Section 3 */}
      <div className="flex flex-col gap-1.5 pt-1">
        <div className="h-1.5 bg-primary/20 rounded-full w-10" />
        <div className="flex gap-1 flex-wrap">
          {[10, 14, 8, 12, 9, 13].map((w, i) => (
            <div key={i} className={`h-3 bg-foreground/10 rounded-full`} style={{ width: `${w * 4}px` }} />
          ))}
        </div>
      </div>

      {/* Draft label */}
      <div className="mt-auto flex items-center justify-between">
        <span className="text-[9px] font-mono text-muted-foreground/60 tracking-wider uppercase">
          — Draft
        </span>
        <span className="text-[9px] font-mono text-muted-foreground/40 tracking-wider">
          {templateId}
        </span>
      </div>
    </div>
  );
}

// ── CvThumbnailCanvas — renders page 1 of the generated PDF ──────────────────

function CvThumbnailCanvas({ pdfUrl }: { pdfUrl: string }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [hasError, setHasError] = useState(false);

  const renderPdf = useCallback(async (url: string) => {
    setIsLoading(true);
    setHasError(false);
    try {
      if (!pdfjsLib) {
        pdfjsLib = await import("pdfjs-dist");
        pdfjsLib.GlobalWorkerOptions.workerSrc = "/pdf.worker.min.mjs";
      }

      const pdf = await pdfjsLib.getDocument(url).promise;
      const page = await pdf.getPage(1);

      const canvas = canvasRef.current;
      if (!canvas) throw new Error("Canvas not available");

      const containerWidth = canvas.parentElement?.clientWidth ?? 280;
      const unscaledVp = page.getViewport({ scale: 1 });
      const scale = containerWidth / unscaledVp.width;
      const viewport = page.getViewport({ scale });

      canvas.height = viewport.height;
      canvas.width = viewport.width;

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
    <div className="w-full h-full relative overflow-hidden">
      {isLoading && (
        <div className="absolute inset-0 flex items-center justify-center bg-muted/30 animate-pulse">
          <div className="h-4 w-4 rounded-full border-2 border-primary/40 border-t-transparent animate-spin" />
        </div>
      )}
      <canvas
        ref={canvasRef}
        className={cn("w-full object-cover", (isLoading || hasError) && "invisible")}
      />
      {!isLoading && hasError && (
        <div className="absolute inset-0 flex items-center justify-center bg-muted/20 text-xs text-muted-foreground font-mono">
          Preview unavailable
        </div>
      )}
    </div>
  );
}

// ── Animation variants ────────────────────────────────────────────────────────

const contentVariants: Variants = {
  hidden: {},
  show: { transition: { staggerChildren: 0.06 } },
};

const itemVariants: Variants = {
  hidden: { opacity: 0, y: 10 },
  show: { opacity: 1, y: 0, transition: { duration: 0.25, ease: "easeOut" } },
};

// ── ProjectCard ───────────────────────────────────────────────────────────────

interface ProjectCardProps {
  project: CvProject;
  onDelete: (id: string) => void;
  variants?: Variants;
}

export function ProjectCard({ project, onDelete, variants }: ProjectCardProps) {
  const router = useRouter();
  const [pdfUrl, setPdfUrl] = useState<string | null>(null);

  // Fetch render job status once on mount — sets PDF URL if render is done
  useEffect(() => {
    if (!project.current_resume_id) return;
    api.getResumeRenderJob(project.current_resume_id).then((job) => {
      if (job?.status === "done") {
        setPdfUrl(api.getPdfUrl(job.job_id));
      }
    });
  }, [project.current_resume_id]);

  const isGenerated = Boolean(project.current_resume_id);
  const docLabel = project.document_type === "cv" ? "Multi-page CV" : "Single Page";
  const typeTag = project.document_type === "cv" ? "CV" : "Resume";

  return (
    <motion.div
      variants={variants}
      whileHover={{
        scale: 1.03,
        boxShadow: "0px 12px 32px -6px hsl(var(--foreground) / 0.12)",
        transition: { type: "spring", stiffness: 300, damping: 20 },
      }}
      onClick={() => router.push(`/editor/${project.id}`)}
      className="group overflow-hidden rounded-2xl border border-border/60 bg-card cursor-pointer"
    >
      {/* ── Preview section ─────────────────────────────────────────────── */}
      <div className="relative h-48 overflow-hidden rounded-t-2xl bg-muted/10">
        {pdfUrl ? (
          <CvThumbnailCanvas pdfUrl={pdfUrl} />
        ) : (
          <CvPlaceholder name={project.name} templateId={project.template_id} />
        )}

        {/* Top-left: document type */}
        <div className="absolute top-2.5 left-2.5">
          <span className="text-[10px] font-mono font-medium px-2 py-0.5 rounded-full bg-background/70 backdrop-blur-sm border border-border/50 text-foreground/80">
            {typeTag}
          </span>
        </div>

        {/* Top-right: template */}
        <div className="absolute top-2.5 right-2.5">
          <span className="text-[10px] font-mono px-2 py-0.5 rounded-full bg-background/70 backdrop-blur-sm border border-border/50 text-muted-foreground">
            {project.template_id}
          </span>
        </div>

        {/* Bottom-left: generated/draft badge */}
        <div className="absolute bottom-2.5 left-2.5">
          <span
            className={cn(
              "text-[10px] font-medium px-2 py-0.5 rounded-full",
              isGenerated
                ? "bg-emerald-500/15 text-emerald-500 dark:bg-emerald-400/15 dark:text-emerald-400 border border-emerald-500/20"
                : "bg-muted/80 text-muted-foreground border border-border/50"
            )}
          >
            {isGenerated ? "Generated" : "Draft"}
          </span>
        </div>
      </div>

      {/* ── Content section ─────────────────────────────────────────────── */}
      <motion.div
        variants={contentVariants}
        initial="hidden"
        animate="show"
        className="p-4 space-y-3"
      >
        {/* Title row */}
        <motion.div
          variants={itemVariants}
          className="flex items-start justify-between gap-2"
        >
          <h3 className="font-mono font-semibold text-sm truncate group-hover:text-primary transition-colors duration-200">
            {project.name}
          </h3>
          <button
            onClick={(e) => {
              e.stopPropagation();
              if (window.confirm(`Delete "${project.name}"? This cannot be undone.`)) {
                onDelete(project.id);
              }
            }}
            className="opacity-0 group-hover:opacity-100 transition-opacity shrink-0 p-1 rounded text-muted-foreground hover:text-destructive"
            aria-label="Delete project"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </motion.div>

        {/* Date row */}
        <motion.div
          variants={itemVariants}
          className="flex items-center gap-1.5 text-[11px] text-muted-foreground"
        >
          <Clock className="h-3 w-3 shrink-0" />
          {formatDate(project.updated_at)}
        </motion.div>

        {/* Footer row */}
        <motion.div
          variants={itemVariants}
          className="flex items-center justify-between pt-0.5"
        >
          <span className="text-[11px] font-mono text-primary/70">{docLabel}</span>
          <button
            onClick={(e) => {
              e.stopPropagation();
              router.push(`/editor/${project.id}`);
            }}
            className="flex items-center gap-1 text-[11px] font-medium text-muted-foreground hover:text-foreground transition-colors group/open"
          >
            Open
            <ArrowRight className="h-3 w-3 transition-transform group-hover/open:translate-x-0.5" />
          </button>
        </motion.div>
      </motion.div>
    </motion.div>
  );
}
