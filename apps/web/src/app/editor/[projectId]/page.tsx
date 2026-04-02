"use client";

// Project-scoped editor page.
//
// Layout:
//   [Action bar: project name | template badge | Analyze Fit | Generate Resume]
//   ┌──────────────────────┬─────────────────────────────────────┐
//   │ Left pane (50%)      │ Right pane (50%)                    │
//   │ Tabs:                │ IF bullets.length === 0:            │
//   │  [Job & Fit]         │   <TemplateThumbnailPreview>        │
//   │  [Bullets (N)]       │ ELSE:                               │
//   │                      │   <PdfPreview> (PDF.js)             │
//   └──────────────────────┴─────────────────────────────────────┘
//
// Two-step JD workflow:
//   1. "Analyze Fit" → runs LlmFitScorer (with hash-based server cache)
//   2. "Generate Resume" → full pipeline (generate + render)
//
// Auto-behaviours:
//   - Navigating to a new projectId resets all project-scoped store state immediately
//   - JD text is re-populated from project.last_jd_text after project data loads
//   - After generation, left tab auto-switches to "Bullets"
//   - "Analyze Fit" button label changes to "Re-analyze Fit" when context has changed

import { useEffect, useState } from "react";
import { useParams, useRouter } from "next/navigation";
import { AlertCircle, X, Download, Loader2 } from "lucide-react";
import { JdInput } from "@/components/editor/JdInput";
import { BulletList } from "@/components/editor/BulletList";
import { FitReportPanel } from "@/components/editor/FitReportPanel";
import { PdfPreview } from "@/components/pdf/PdfPreview";
import { StaticPdfPreview } from "@/components/pdf/StaticPdfPreview";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Tabs,
  TabsList,
  TabsTrigger,
  TabsContent,
} from "@/components/ui/tabs";
import { useResumeStore } from "@/store/resumeStore";
import { useProjectStore } from "@/store/projectStore";
import { api } from "@/lib/api";

// ── Main page ─────────────────────────────────────────────────────────────────

export default function ProjectEditorPage() {
  const params = useParams();
  const router = useRouter();
  const projectId = params.projectId as string;

  const {
    generate,
    analyzeFit,
    autoLoadCachedFitScore,
    loadResume,
    isGenerating,
    fitScoreLoading,
    fitScoreCacheHit,
    contextChangedSinceAnalysis,
    fitReport,
    error,
    clearError,
    bullets,
    resumeId,
    jdText,
    setJdText,
    resetForProject,
    renderStatus,
    rerender,
    renderJobId,
  } = useResumeStore();
  const { currentProject, loadProject, loadTemplates } = useProjectStore();

  const [leftTab, setLeftTab] = useState<"jd" | "bullets">("jd");
  const [isDownloading, setIsDownloading] = useState(false);

  const handleDownload = async () => {
    if (!renderJobId) return;
    setIsDownloading(true);
    try {
      await api.downloadPdf(renderJobId);
    } catch (e) {
      console.error("[Editor] Download failed:", e);
    } finally {
      setIsDownloading(false);
    }
  };

  // Effect 1: Reset all project-scoped state immediately on project navigation.
  // This prevents state bleed-through when switching between projects.
  useEffect(() => {
    resetForProject(projectId);
  // resetForProject is a stable Zustand action — safe to omit from deps
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId]);

  // Load project data + templates on mount / projectId change
  useEffect(() => {
    loadProject(projectId);
    loadTemplates();
  }, [projectId, loadProject, loadTemplates]);

  // Effect 2: Populate JD + restore saved resume after project data loads.
  // Only runs when the loaded project matches this page's projectId.
  useEffect(() => {
    if (currentProject?.id !== projectId) return;

    if (currentProject.last_jd_text) {
      setJdText(currentProject.last_jd_text);
      autoLoadCachedFitScore(currentProject.last_jd_text);
    }

    // Restore previously-generated bullets from DB if the store is empty.
    // This handles page refresh — bullets are in resume_bullets, not just memory.
    if (currentProject.current_resume_id && !resumeId) {
      loadResume(currentProject.current_resume_id);
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentProject?.id, currentProject?.last_jd_text, currentProject?.current_resume_id, projectId]);

  // Effect 3: Auto-switch to bullets tab after generation completes.
  useEffect(() => {
    if (bullets.length > 0) setLeftTab("bullets");
  }, [bullets.length]);

  // We only need the template id slug to build the render-pdf URL.
  const templateId = currentProject?.template_id ?? null;

  // Debug: log which right-pane scenario is active.
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

  // Derive "Analyze Fit" button label based on current state
  const analyzeFitLabel = fitScoreLoading
    ? "Analyzing..."
    : contextChangedSinceAnalysis && fitReport
    ? "Re-analyze Fit"
    : "Analyze Fit";

  return (
    <div className="flex flex-col h-[calc(100vh-53px)] bg-background">
      {/* Action bar — project context + two-button workflow */}
      <div className="flex items-center justify-between px-6 py-2.5 border-b bg-background/90 backdrop-blur-sm shrink-0">
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

        {/* Two-step action bar */}
        <div className="flex items-center gap-2">
          {bullets.length > 0 && (
            <span className="text-xs text-muted-foreground shrink-0">
              {bullets.length} bullets
            </span>
          )}
          <Button
            variant="outline"
            size="sm"
            onClick={() => analyzeFit(false)}
            disabled={fitScoreLoading || isGenerating || !jdText.trim()}
          >
            {analyzeFitLabel}
          </Button>
          {bullets.length > 0 && (
            <Button
              variant="outline"
              size="sm"
              onClick={() => rerender()}
              disabled={
                renderStatus === "queued" ||
                renderStatus === "rendering" ||
                isGenerating
              }
            >
              {renderStatus === "queued" || renderStatus === "rendering"
                ? "Rendering..."
                : "Render PDF"}
            </Button>
          )}
          {renderStatus === "done" && renderJobId && (
            <Button
              size="sm"
              variant="outline"
              onClick={handleDownload}
              disabled={isDownloading}
              className="gap-1.5"
            >
              {isDownloading
                ? <Loader2 className="h-3.5 w-3.5 animate-spin" />
                : <Download className="h-3.5 w-3.5" />}
              {isDownloading ? "Downloading..." : "Download PDF"}
            </Button>
          )}
          <Button
            size="sm"
            onClick={() => generate(projectId)}
            disabled={isGenerating || fitScoreLoading || !jdText.trim()}
          >
            {isGenerating ? "Generating..." : "Generate Resume"}
          </Button>
        </div>
      </div>

      {/* Error banner */}
      {error && (
        <div className="mx-6 mt-2 px-3 py-2 bg-destructive/10 text-destructive text-sm rounded-md flex items-center justify-between shrink-0">
          <div className="flex items-center gap-2">
            <AlertCircle className="h-4 w-4 shrink-0" />
            <span>{error}</span>
          </div>
          <button
            onClick={clearError}
            className="ml-4 hover:opacity-70 transition-opacity"
            aria-label="Dismiss error"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      )}

      {/* Split pane */}
      <div className="flex flex-1 overflow-hidden">
        {/* Left: tabbed pane — Job & Fit / Bullets */}
        <div className="w-1/2 border-r flex flex-col overflow-hidden">
          <Tabs
            value={leftTab}
            onValueChange={(v) => setLeftTab(v as "jd" | "bullets")}
            className="flex flex-col flex-1 overflow-hidden"
          >
            <div className="px-4 pt-3 shrink-0 border-b">
              <TabsList className="w-full">
                <TabsTrigger value="jd" className="flex-1">
                  Job &amp; Fit
                </TabsTrigger>
                <TabsTrigger value="bullets" className="flex-1">
                  Bullets
                  {bullets.length > 0 && (
                    <span className="ml-1.5 text-xs text-muted-foreground">
                      ({bullets.length})
                    </span>
                  )}
                </TabsTrigger>
              </TabsList>
            </div>

            <TabsContent value="jd" className="flex-1 overflow-hidden m-0">
              <ScrollArea className="h-full">
                <div className="p-4 flex flex-col gap-4">
                  <JdInput projectId={projectId} />
                  <FitReportPanel />
                </div>
              </ScrollArea>
            </TabsContent>

            <TabsContent value="bullets" className="flex-1 overflow-hidden m-0">
              <ScrollArea className="h-full">
                <div className="p-4">
                  <BulletList />
                </div>
              </ScrollArea>
            </TabsContent>
          </Tabs>
        </div>

        {/* Right: template PDF preview (pre-generation) or PDF.js live preview (post-generation) */}
        <div className="w-1/2 bg-muted/30 relative overflow-hidden">
          {bullets.length > 0 ? (
            // Post-generation: PDF.js live preview (debounced at 300ms)
            <PdfPreview />
          ) : templateId ? (
            // Pre-generation: compiled PDF preview of the selected template.
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
