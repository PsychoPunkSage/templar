"use client";

// Project-scoped editor page.
//
// Layout:
//   [Action bar: project name | template badge | Generate button]
//   ┌──────────────────────┬─────────────────────────────────────┐
//   │ Left pane (50%)      │ Right pane (50%)                    │
//   │ - JdInput            │ IF bullets.length === 0:            │
//   │ - FitReportPanel     │   <TemplateThumbnailPreview>        │
//   │ - BulletList         │ ELSE:                               │
//   │                      │   <PdfPreview> (PDF.js)             │
//   └──────────────────────┴─────────────────────────────────────┘
//
// Context tab removed from this editor — context management lives at /context.

import { useEffect } from "react";
import { useParams, useRouter } from "next/navigation";
import { JdInput } from "@/components/editor/JdInput";
import { BulletList } from "@/components/editor/BulletList";
import { FitReportPanel } from "@/components/editor/FitReportPanel";
import { PdfPreview } from "@/components/pdf/PdfPreview";
import { StaticPdfPreview } from "@/components/pdf/StaticPdfPreview";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useResumeStore } from "@/store/resumeStore";
import { useProjectStore } from "@/store/projectStore";
import { api } from "@/lib/api";

// ── Main page ─────────────────────────────────────────────────────────────────

export default function ProjectEditorPage() {
  const params = useParams();
  const router = useRouter();
  const projectId = params.projectId as string;

  const { generate, isGenerating, error, clearError, bullets, jdText } = useResumeStore();
  const { currentProject, loadProject, loadTemplates } = useProjectStore();

  // Load project data + templates on mount
  useEffect(() => {
    loadProject(projectId);
    loadTemplates();
  }, [projectId, loadProject, loadTemplates]);

  // We only need the template id slug to build the render-pdf URL.
  // currentProject.template_id IS the slug (e.g. "generic-cv") — we do not
  // need to look it up in the templates array, which avoids a race where
  // loadTemplates resolves after loadProject and StaticPdfPreview never mounts.
  const templateId = currentProject?.template_id ?? null;

  // Debug: log which right-pane scenario is active whenever the discriminating
  // variables change. Diagnostic only — remove once root cause is found.
  useEffect(() => {
    if (bullets.length > 0) {
      console.log("[Editor] Right pane → Scenario A: PdfPreview (live render)", {
        bullets: bullets.length,
        templateId,
      });
    } else if (templateId) {
      console.log("[Editor] Right pane → Scenario B: StaticPdfPreview (template thumbnail)", {
        templateId,
        pdfUrl: api.getTemplateRenderPdfUrl(templateId),
      });
    } else {
      console.log("[Editor] Right pane → Scenario C: Placeholder (no project or template yet)", {
        currentProject,
        templateId,
      });
    }
  }, [bullets.length, templateId, currentProject]);

  const handleGenerate = () => {
    // Pass projectId so the store knows which project to link the resume to
    generate(projectId);
  };

  return (
    <div className="flex flex-col h-[calc(100vh-53px)] bg-background">
      {/* Action bar — project context + generate button */}
      <div className="flex items-center justify-between px-6 py-2.5 border-b bg-background shrink-0">
        <div className="flex items-center gap-3 min-w-0">
          {/* Back to projects */}
          <button
            onClick={() => router.push("/")}
            className="text-muted-foreground hover:text-foreground transition-colors text-sm"
            aria-label="Back to projects"
          >
            Projects
          </button>
          <span className="text-muted-foreground">/</span>
          <span className="font-medium text-sm truncate">
            {currentProject?.name ?? "Loading..."}
          </span>
          {currentProject && (
            <span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
              {currentProject.template_id}
            </span>
          )}
        </div>

        <div className="flex items-center gap-2">
          {bullets.length > 0 && (
            <span className="text-xs text-muted-foreground shrink-0">
              {bullets.length} bullets
            </span>
          )}
          <Button
            onClick={handleGenerate}
            disabled={isGenerating || !jdText.trim()}
            size="sm"
          >
            {isGenerating ? "Generating..." : "Generate Resume"}
          </Button>
        </div>
      </div>

      {/* Error banner */}
      {error && (
        <div className="mx-6 mt-2 px-3 py-2 bg-destructive/10 text-destructive text-sm rounded-md flex items-center justify-between shrink-0">
          <span>{error}</span>
          <button
            onClick={clearError}
            className="font-bold ml-4 hover:opacity-70 transition-opacity"
            aria-label="Dismiss error"
          >
            x
          </button>
        </div>
      )}

      {/* Split pane */}
      <div className="flex flex-1 overflow-hidden">
        {/* Left: JD input + fit report + bullets */}
        <div className="w-1/2 border-r flex flex-col overflow-hidden">
          <ScrollArea className="flex-1">
            <div className="p-4 flex flex-col gap-4">
              <JdInput />
              <FitReportPanel />
              <BulletList />
            </div>
          </ScrollArea>
        </div>

        {/* Right: template PDF preview (pre-generation) or PDF.js live preview (post-generation) */}
        <div className="w-1/2 bg-muted/30 relative overflow-hidden">
          {bullets.length > 0 ? (
            // Post-generation: PDF.js live preview (debounced at 300ms)
            <PdfPreview />
          ) : templateId ? (
            // Pre-generation: compiled PDF preview of the selected template.
            // Uses GET /api/v1/templates/:id/render-pdf — Tectonic compiles the
            // template once; subsequent loads are served from the in-process cache.
            // We use templateId (= currentProject.template_id) directly — no need
            // to wait for the templates list to resolve before mounting.
            <StaticPdfPreview
              pdfUrl={api.getTemplateRenderPdfUrl(templateId)}
            />
          ) : (
            // Project not yet loaded — show neutral placeholder
            <div className="flex h-full items-center justify-center text-muted-foreground text-sm">
              PDF preview will appear here after generation.
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
