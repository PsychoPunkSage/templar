"use client";

// Project-scoped editor page.
//
// Layout:
//   [Action bar: project name | template badge | Analyze Fit | Generate Resume]
//   ┌──────────────────────┬─────────────────────────────────────┐
//   │ Left pane (50%)      │ Right pane (50%)                    │
//   │ Tabs:                │ IF entryGroups.length === 0:        │
//   │  [Job & Fit]         │   <TemplateThumbnailPreview>        │
//   │  [Bullets (N)]       │ ELSE:                               │
//   │                      │   <PdfPreview> (PDF.js)             │
//   └──────────────────────┴─────────────────────────────────────┘
//
// Two-step JD workflow:
//   1. "Analyze Fit" → runs LlmFitScorer (with hash-based server cache)
//   2. "Generate Resume" → async job enqueue + poll (FIX-08)
//
// Auto-behaviours:
//   - Navigating to a new projectId resets all project-scoped store state immediately
//   - JD text is re-populated from project.last_jd_text after project data loads
//   - On load: if project.generation_job_id is set and job is in-flight, polling resumes
//   - After generation, left tab auto-switches to "Bullets"
//   - "Analyze Fit" button label changes to "Re-analyze Fit" when context has changed

import { useEffect, useState } from "react";
import { useParams, useRouter } from "next/navigation";
import { AlertCircle, X, Download, Loader2, Brain } from "lucide-react";
import { JdInput } from "@/components/editor/JdInput";
import { BulletList } from "@/components/editor/BulletList";
import { FitReportPanel } from "@/components/editor/FitReportPanel";
import { PdfPreview } from "@/components/pdf/PdfPreview";
import { StaticPdfPreview } from "@/components/pdf/StaticPdfPreview";
import { CoverLetterPane } from "@/components/editor/CoverLetterPane";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Tabs,
  TabsList,
  TabsTrigger,
  TabsContent,
} from "@/components/ui/tabs";
import { useResumeStore, MVP_USER_ID } from "@/store/resumeStore";
import { useProjectStore } from "@/store/projectStore";
import { useCoverLetterStore } from "@/store/coverLetterStore";
import { useAuthStore } from "@/store/authStore";
import { PersonaSelect } from "@/components/editor/PersonaSelect";
import { BookOpen } from "lucide-react";
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
    pollGenerationStatus,
    isGenerating,
    fitScoreLoading,
    fitScoreCacheHit,
    contextChangedSinceAnalysis,
    fitReport,
    error,
    clearError,
    entryGroups,
    resumeId,
    jdText,
    setJdText,
    resetForProject,
    renderStatus,
    rerender,
    renderJobId,
    generationStatus,
    hydrateQueue,
    refinementQueue,
    refiningBullets,
    applyQueue,
    clearQueue,
    pageCount,
  } = useResumeStore();
  const { currentProject, loadProject, loadTemplates } = useProjectStore();

  const [leftTab, setLeftTab] = useState<"jd" | "bullets">("jd");
  const [rightPane, setRightPane] = useState<"resume" | "cover_letter">("resume");
  const [isDownloading, setIsDownloading] = useState(false);
  const [selectedPersonaId, setSelectedPersonaId] = useState<string | null>(null);

  const { coverId, generatedWithJdText } = useCoverLetterStore();
  const clIsStale =
    !!generatedWithJdText && !!coverId && generatedWithJdText !== jdText;

  // Total bullet count across all entry groups
  const bulletCount = entryGroups.reduce((acc, g) => acc + g.bullets.length, 0);
  const hasBullets = bulletCount > 0;

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
  // Also hydrates the refinement queue from localStorage for this project.
  useEffect(() => {
    // Only reset when switching to a different project.
    // Returning to the same project (e.g. editor → interview → back) preserves state.
    if (useResumeStore.getState().currentProjectId !== projectId) {
      resetForProject(projectId);
      useCoverLetterStore.getState().reset();
    }
    hydrateQueue(projectId);
  // resetForProject, hydrateQueue, and coverLetterStore.reset are stable Zustand actions
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

    const genJobId = currentProject.generation_job_id;
    const savedResumeId = currentProject.current_resume_id;

    if (genJobId && !resumeId) {
      // FIX-10: Prefer the status endpoint when a generation_job_id exists.
      // It carries full typed EntryGroup display headers (company/role/dates).
      // Only fall back to loadResume() if the status endpoint fails or has no entry_groups.
      api.getGenerationStatus(genJobId).then((status) => {
        if (status.status === "done" && status.entry_groups?.length) {
          // Full typed display headers available — populate directly, no loadResume needed
          const resId = status.resume_id ?? null;
          useResumeStore.setState({
            generationJobId: genJobId,
            resumeId: resId,
            entryGroups: status.entry_groups,
            fitReport: status.fit_report ?? null,
            generationStatus: "done",
            isGenerating: false,
          });
          console.log("[Editor] Restored generation from job (full headers)", genJobId);
          // Also restore render state — getGenerationStatus doesn't carry renderJobId
          if (resId) {
            api.getResumeRenderJob(resId).then((renderJob) => {
              if (!renderJob) return;
              if (renderJob.status === "done") {
                useResumeStore.setState({ renderJobId: renderJob.job_id, renderStatus: "done" });
              } else if (renderJob.status === "processing" || renderJob.status === "queued") {
                useResumeStore.setState({
                  renderJobId: renderJob.job_id,
                  renderStatus: renderJob.status === "processing" ? "rendering" : "queued",
                });
                useResumeStore.getState().pollRenderStatus();
              }
            }).catch(() => {});
          }
        } else if (status.status === "done" && savedResumeId) {
          // Status done but no entry_groups in result JSONB — fall back to loadResume.
          // loadResume uses resumes.entry_groups (post-migration 014) or entry_header labels.
          loadResume(savedResumeId);
        } else if (status.status === "queued" || status.status === "processing") {
          // Job still running — resume the polling loop
          useResumeStore.setState({
            generationJobId: genJobId,
            generationStatus: status.status as "queued" | "processing",
            isGenerating: true,
          });
          useResumeStore.getState().pollGenerationStatus();
          console.log("[Editor] Resumed generation polling for job", genJobId, status.status);
        } else if (savedResumeId) {
          // Failed or unknown — restore bullets from DB
          loadResume(savedResumeId);
        }
      }).catch(() => {
        // Network error or job not found — try to restore from DB
        if (savedResumeId && !resumeId) loadResume(savedResumeId);
      });
    } else if (savedResumeId && !resumeId) {
      // No generation_job_id (old resume) — only path is loadResume
      loadResume(savedResumeId);
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentProject?.id, currentProject?.last_jd_text, currentProject?.current_resume_id,
      currentProject?.generation_job_id, projectId]);

  // Effect 3: Auto-switch to bullets tab after generation completes.
  useEffect(() => {
    if (hasBullets) setLeftTab("bullets");
  }, [hasBullets]);

  // Effect 4: Auto-load the most recent cover letter once resumeId is available.
  // Skips if a cover letter is already loaded in this session (coverId is set).
  useEffect(() => {
    if (!resumeId || coverId) return;
    const userId = useAuthStore.getState().internalUserId ?? MVP_USER_ID;
    useCoverLetterStore.getState().loadCoverLetterForResume(userId, resumeId);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resumeId, coverId]);

  // We only need the template id slug to build the render-pdf URL.
  const templateId = currentProject?.template_id ?? null;

  // Debug: log which right-pane scenario is active.
  useEffect(() => {
    if (hasBullets) {
      console.log("[Editor] Right pane → Scenario A: PdfPreview (live render)", {
        bullets: bulletCount,
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
  }, [hasBullets, bulletCount, templateId, currentProject]);

  // Derive button labels based on current state
  const analyzeFitLabel = fitScoreLoading
    ? "Analyzing..."
    : contextChangedSinceAnalysis && fitReport
    ? "Re-analyze Fit"
    : "Analyze Fit";

  // FIX-08: Show generation phase in button label
  const generateLabel = isGenerating
    ? generationStatus === "queued"
      ? "Queued..."
      : "Generating..."
    : "Generate Resume";

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
          {/* CV badge — shown when document_type is 'cv' */}
          {currentProject?.document_type === "cv" && (
            <span className="shrink-0 flex items-center gap-1 rounded-full bg-indigo-100 dark:bg-indigo-900/40 px-2 py-0.5 text-xs text-indigo-700 dark:text-indigo-300 font-medium">
              <BookOpen className="h-3 w-3" />
              CV
            </span>
          )}
          {/* Page count badge — shown after generation completes for CV mode */}
          {pageCount !== null && pageCount > 1 && (
            <span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
              {pageCount} pages
            </span>
          )}
        </div>

        {/* Two-step action bar */}
        <div className="flex items-center gap-2">
          <PersonaSelect value={selectedPersonaId} onChange={setSelectedPersonaId} />
          {hasBullets && (
            <span className="text-xs text-muted-foreground shrink-0">
              {bulletCount} bullets
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
          {hasBullets && (
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
          {renderStatus === "done" && (
            <Button
              size="sm"
              variant="outline"
              onClick={() => router.push(`/interview/${projectId}`)}
              className="gap-1.5"
              title="Prepare for interviews using your resume"
            >
              <Brain className="h-3.5 w-3.5" />
              Prep Interview
            </Button>
          )}
          <Button
            size="sm"
            onClick={() => generate(projectId, selectedPersonaId)}
            disabled={isGenerating || fitScoreLoading || !jdText.trim()}
          >
            {isGenerating && <Loader2 className="h-3.5 w-3.5 animate-spin mr-1.5" />}
            {generateLabel}
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
                  {hasBullets && (
                    <span className="ml-1.5 text-xs text-muted-foreground">
                      ({bulletCount})
                    </span>
                  )}
                  {isGenerating && !hasBullets && (
                    <Loader2 className="ml-1.5 h-3 w-3 animate-spin text-muted-foreground" />
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

            <TabsContent value="bullets" className="flex-1 overflow-hidden m-0 flex flex-col">
              <ScrollArea className="flex-1 min-h-0">
                <div className="p-4">
                  <BulletList />
                </div>
              </ScrollArea>
              {/* Queue bar — outside ScrollArea so it stays pinned to bottom of pane */}
              {refinementQueue.length > 0 && (
                <div className="flex items-center justify-between gap-2 px-3 py-2 border-t border-border bg-background/95 backdrop-blur-sm shrink-0">
                  <span className="text-xs text-muted-foreground">
                    {refiningBullets.length > 0 ? (
                      <span className="flex items-center gap-1.5">
                        <Loader2 className="h-3 w-3 animate-spin" />
                        Applying...
                      </span>
                    ) : (
                      `${refinementQueue.length} change${refinementQueue.length > 1 ? "s" : ""} queued`
                    )}
                  </span>
                  <div className="flex gap-1.5">
                    <button
                      onClick={clearQueue}
                      disabled={refiningBullets.length > 0}
                      className="text-xs text-muted-foreground hover:text-foreground disabled:opacity-40 transition-colors"
                    >
                      Clear
                    </button>
                    <button
                      onClick={() => applyQueue()}
                      disabled={refiningBullets.length > 0}
                      className="text-xs px-2.5 py-1 rounded bg-primary text-primary-foreground disabled:opacity-40 hover:opacity-90 transition-opacity"
                    >
                      Apply All →
                    </button>
                  </div>
                </div>
              )}
            </TabsContent>
          </Tabs>
        </div>

        {/* Right: toggle header + resume preview or cover letter pane */}
        <div className="w-1/2 bg-muted/30 relative overflow-hidden flex flex-col">
          {/* Toggle */}
          <div className="flex items-center px-3 py-1.5 border-b bg-background/80 shrink-0">
            <div className="flex rounded-md border overflow-hidden text-xs">
              <button
                onClick={() => setRightPane("resume")}
                className={`px-3 py-1 transition-colors ${
                  rightPane === "resume"
                    ? "bg-primary text-primary-foreground"
                    : "text-muted-foreground hover:text-foreground"
                }`}
              >
                Resume
              </button>
              <button
                onClick={() => setRightPane("cover_letter")}
                className={`px-3 py-1 transition-colors flex items-center gap-1 ${
                  rightPane === "cover_letter"
                    ? "bg-primary text-primary-foreground"
                    : "text-muted-foreground hover:text-foreground"
                }`}
              >
                Cover Letter
                {clIsStale && (
                  <span className="h-1.5 w-1.5 rounded-full bg-amber-400 inline-block" />
                )}
              </button>
            </div>
          </div>

          {/* Pane content */}
          <div className="flex-1 overflow-hidden">
            {rightPane === "resume" ? (
              hasBullets ? (
                <PdfPreview />
              ) : templateId ? (
                <StaticPdfPreview pdfUrl={api.getTemplateRenderPdfUrl(templateId)} />
              ) : (
                <div className="flex h-full items-center justify-center text-muted-foreground text-sm">
                  PDF preview will appear here after generation.
                </div>
              )
            ) : (
              <CoverLetterPane
                jdText={jdText}
                resumeId={resumeId}
                personaId={selectedPersonaId}
              />
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
