"use client";

import { create } from "zustand";
import { api } from "@/lib/api";
import type { SimulatedBullet, FitReport, AuditManifest } from "@templar/types";

/**
 * Hardcoded user ID for MVP development.
 * Auth integration (Clerk) will replace this in Phase 9.
 */
export const MVP_USER_ID = "00000000-0000-0000-0000-000000000001";

type RenderStatus = "idle" | "queued" | "rendering" | "done" | "failed";

interface ResumeStore {
  // ─── State ─────────────────────────────────────────────────────────────────
  resumeId: string | null;
  bullets: SimulatedBullet[];
  jdText: string;
  fitReport: FitReport | null;
  auditManifest: AuditManifest | null;
  renderJobId: string | null;
  renderStatus: RenderStatus;
  isGenerating: boolean;
  error: string | null;
  /** The project this generation belongs to. Used to link resume after generation. */
  currentProjectId: string | null;
  /** True while analyzeFit() is in flight. */
  fitScoreLoading: boolean;
  /** null = no fit score loaded yet; true = came from cache; false = freshly scored via LLM. */
  fitScoreCacheHit: boolean | null;
  /** SHA-256 hash of the last JD text analysed. */
  lastJdHash: string | null;
  /** SHA-256 hash of the context snapshot used for the last fit score. */
  lastContextHash: string | null;
  /** The JD text that was in use the last time analyzeFit() completed successfully. */
  lastAnalyzedJdText: string | null;
  /** True when context has been updated since the last successful fit analysis. */
  contextChangedSinceAnalysis: boolean;

  // ─── Actions ───────────────────────────────────────────────────────────────
  setJdText: (text: string) => void;
  setCurrentProjectId: (id: string | null) => void;
  /** Reset all project-scoped state when navigating to a different project. */
  resetForProject: (projectId: string) => void;
  /** Mark fit score as stale (e.g., after a context entry is updated). */
  invalidateFitScore: () => void;
  /**
   * Silent background cache restore on page load.
   * Calls POST /api/v1/resumes/fit-score/cached with the restored JD text.
   * On a cache hit, populates fitReport without setting fitScoreLoading —
   * so there is no loading spinner; it just appears.
   * On a miss (404) or any error, silently no-ops — the user sees an empty fit panel.
   */
  autoLoadCachedFitScore: (jdText: string) => Promise<void>;
  /**
   * Full generation pipeline:
   * 1. POST /api/v1/resumes/generate
   * 2. GET  /api/v1/resumes/:id/audit  (non-blocking)
   * 3. POST /api/v1/render
   * 4. Start polling render status
   * 5. Link resume to project (fire-and-forget)
   */
  generate: (projectId?: string) => Promise<void>;
  /**
   * Standalone fit analysis (two-step JD workflow).
   * Calls POST /api/v1/resumes/fit-score and updates fitReport, fitScoreCacheHit, hashes.
   * pass forceRefresh=true to bypass server-side cache.
   */
  analyzeFit: (forceRefresh?: boolean) => Promise<void>;
  /** Polls the render job status every 2 seconds until done or failed. */
  pollRenderStatus: () => void;
  clearError: () => void;
}

export const useResumeStore = create<ResumeStore>((set, get) => ({
  resumeId: null,
  bullets: [],
  jdText: "",
  fitReport: null,
  auditManifest: null,
  renderJobId: null,
  renderStatus: "idle",
  isGenerating: false,
  error: null,
  currentProjectId: null,
  fitScoreLoading: false,
  fitScoreCacheHit: null,
  lastJdHash: null,
  lastContextHash: null,
  lastAnalyzedJdText: null,
  contextChangedSinceAnalysis: false,

  setJdText: (text) => set({ jdText: text }),
  setCurrentProjectId: (id) => set({ currentProjectId: id }),
  clearError: () => set({ error: null }),

  resetForProject: (projectId) => set({
    currentProjectId: projectId,
    jdText: "",
    fitReport: null,
    fitScoreCacheHit: null,
    lastJdHash: null,
    lastContextHash: null,
    lastAnalyzedJdText: null,
    contextChangedSinceAnalysis: false,
    bullets: [],
    resumeId: null,
    auditManifest: null,
    renderJobId: null,
    renderStatus: "idle",
    isGenerating: false,
    error: null,
  }),

  invalidateFitScore: () => set({ contextChangedSinceAnalysis: true }),

  autoLoadCachedFitScore: async (jdText) => {
    if (!jdText.trim()) return;
    try {
      const resp = await api.getCachedFitScore(MVP_USER_ID, jdText);
      if (!resp) return; // cache miss — silent no-op
      set({
        fitReport: resp.fit_report,
        fitScoreCacheHit: true,
        lastJdHash: resp.jd_hash,
        lastContextHash: resp.context_hash,
        lastAnalyzedJdText: jdText,
        contextChangedSinceAnalysis: false,
      });
    } catch {
      // Network error — treat as miss, don't surface error to user
    }
  },

  analyzeFit: async (forceRefresh = false) => {
    const { jdText, lastAnalyzedJdText, fitReport, contextChangedSinceAnalysis } = get();
    if (!jdText.trim()) return;

    // Skip if: not forced, report exists, JD unchanged, context unchanged
    if (!forceRefresh && fitReport !== null
        && jdText === lastAnalyzedJdText && !contextChangedSinceAnalysis) {
      return;
    }

    set({ fitScoreLoading: true, error: null });
    try {
      const resp = await api.analyzeFit(MVP_USER_ID, jdText, forceRefresh);
      set({
        fitReport: resp.fit_report,
        fitScoreCacheHit: resp.cache_hit,
        lastJdHash: resp.jd_hash,
        lastContextHash: resp.context_hash,
        lastAnalyzedJdText: jdText,
        contextChangedSinceAnalysis: false,
      });
    } catch (e) {
      set({ error: e instanceof Error ? e.message : "Fit analysis failed" });
    } finally {
      set({ fitScoreLoading: false });
    }
  },

  generate: async (projectId?: string) => {
    const { jdText } = get();
    console.log("[store] generate() called", { jdText: jdText.slice(0, 60) });
    if (!jdText.trim()) {
      console.warn("[store] generate() aborted — jdText is empty");
      return;
    }
    console.log("[store] generate() start — proceeding with pipeline");
    // Allow caller to set projectId context for this generation session
    if (projectId !== undefined) set({ currentProjectId: projectId });

    set({
      isGenerating: true,
      error: null,
      renderStatus: "idle",
      renderJobId: null,
      auditManifest: null,
    });

    try {
      // Step 1: Generate resume
      const generated = await api.generateResume(MVP_USER_ID, jdText);
      console.log("[store] Step 1 done — resume generated", { resume_id: generated.resume_id, bulletCount: generated.bullets.length });
      set({
        resumeId: generated.resume_id,
        bullets: generated.bullets,
        fitReport: generated.fit_report,
        fitScoreCacheHit: false,
        lastJdHash: null,
        lastContextHash: null,
      });

      // Step 2: Fetch audit manifest (non-blocking — failure should not abort flow)
      api.getAuditManifest(generated.resume_id).then((audit) => {
        set({ auditManifest: audit });
      }).catch(() => {
        // Audit manifest fetch failure is non-fatal — silently ignored
      });

      // Step 3: Trigger PDF render
      const renderResp = await api.triggerRender(generated.resume_id);
      console.log("[store] Step 3 done — render triggered", { job_id: renderResp.job_id });
      set({ renderJobId: renderResp.job_id, renderStatus: "queued" });

      // Step 4: Start polling
      get().pollRenderStatus();

      // Step 5: Link resume to project (fire-and-forget).
      // We use fire-and-forget here because: if it fails, the user still has their
      // generated resume (resume_id is returned above). current_resume_id is
      // "best effort" bookkeeping — not on the critical render path.
      const pid = get().currentProjectId;
      if (pid) {
        api.updateProject(pid, { current_resume_id: generated.resume_id }).catch((err) => {
          console.warn("Failed to link resume to project (non-fatal):", err);
        });
      }
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : "Generation failed",
      });
    } finally {
      set({ isGenerating: false });
    }
  },

  pollRenderStatus: () => {
    const poll = async () => {
      const { renderJobId, renderStatus } = get();

      // Stop polling if no job or already terminal
      if (!renderJobId || renderStatus === "done" || renderStatus === "failed") {
        return;
      }

      try {
        const statusResp = await api.getRenderStatus(renderJobId);
        // Map backend "processing" → frontend "rendering".
        // The backend DB stores the job lifecycle state as "processing" (RenderStatus::Processing),
        // but the frontend RenderStatus union only has "rendering" — not "processing".
        // Without this mapping the store receives an unrecognised string, the polling
        // condition `newStatus !== "done" && newStatus !== "failed"` still continues,
        // but renderStatus never reaches a value the UI components recognise as active.
        const rawStatus = statusResp.status === "processing" ? "rendering" : statusResp.status;
        const newStatus = rawStatus as RenderStatus;
        console.log("[store] poll tick", { renderJobId, rawStatus: statusResp.status, mappedStatus: newStatus });

        if (newStatus === "failed") {
          // Surface the backend error_message to the UI so the user sees a real
          // error description instead of just "PDF render failed." with no context.
          const backendError = statusResp.error_message
            ? `Render failed: ${statusResp.error_message}`
            : "Resume PDF render failed. Please try generating again.";
          console.log("[store] render terminal", { newStatus });
          set({ renderStatus: "failed", error: backendError });
        } else {
          set({ renderStatus: newStatus });
        }

        if (newStatus === "done") {
          console.log("[store] render terminal", { newStatus });
        }

        if (newStatus !== "done" && newStatus !== "failed") {
          // Continue polling every 2 seconds
          setTimeout(poll, 2000);
        }
      } catch {
        // Network error during poll — mark as failed
        set({
          renderStatus: "failed",
          error: "Lost connection while waiting for render. Please try again.",
        });
      }
    };

    // Initial poll (starts the loop)
    setTimeout(poll, 2000);
  },
}));
