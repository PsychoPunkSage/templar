"use client";

import { create } from "zustand";
import { api } from "@/lib/api";
import { useAuthStore } from "@/store/authStore";
import type {
  SimulatedBullet,
  FitReport,
  AuditManifest,
  ResumeBulletRow,
  EntryGroup,
} from "@templar/types";

type GenerationStatus = "idle" | "queued" | "processing" | "done" | "failed";
type RenderStatus = "idle" | "queued" | "rendering" | "done" | "failed";

export interface RefinementQueueItem {
  bulletText: string;
  sourceEntryId: string;
  section: string;
  instruction: string;
  originalText: string; // for revert on rejection
}

function queueStorageKey(projectId: string): string {
  return `templar:refinement-queue:${projectId}`;
}

function persistQueue(projectId: string, queue: RefinementQueueItem[]): void {
  try {
    localStorage.setItem(queueStorageKey(projectId), JSON.stringify(queue));
  } catch {
    // localStorage unavailable — silently ignore
  }
}

function loadQueue(projectId: string): RefinementQueueItem[] {
  try {
    const raw = localStorage.getItem(queueStorageKey(projectId));
    if (!raw) return [];
    return JSON.parse(raw) as RefinementQueueItem[];
  } catch {
    return [];
  }
}

/**
 * Reconstructs an EntryGroup from ResumeBulletRow for the loadResume() path.
 * Display headers are not available from DB alone — uses "other" fallback with source_entry_id label.
 * Groups bullets by source_entry_id, preserving order_idx / id ordering from the DB query.
 */
function bulletRowsToEntryGroups(rows: ResumeBulletRow[]): EntryGroup[] {
  // Preserve insertion order — rows arrive sorted by (section, order_idx, id) from the DB
  const orderMap = new Map<string, number>();
  const groupMap = new Map<string, EntryGroup>();

  for (const row of rows) {
    if (!groupMap.has(row.source_entry_id)) {
      orderMap.set(row.source_entry_id, orderMap.size);
      groupMap.set(row.source_entry_id, {
        source_entry_id: row.source_entry_id,
        section: row.section,
        // Use stored entry_header LaTeX as label when available (better than raw UUID).
        // Full typed display headers come from detail.resume.entry_groups in loadResume().
        display_header: { type: "other", label: row.entry_header ?? row.source_entry_id },
        entry_header_latex: row.entry_header ?? null,
        bullets: [],
      });
    }

    const bullet: SimulatedBullet = {
      text: row.bullet_text,
      source_entry_id: row.source_entry_id,
      section: row.section,
      verified_line_count: row.line_count as 1 | 2,
      jd_keywords_used: [],
      was_adjusted: false,
      flagged_for_review: row.grounding_score < 0.8 || row.rejection_reason != null,
    };

    groupMap.get(row.source_entry_id)!.bullets.push(bullet);
  }

  // Return in insertion order
  return [...orderMap.entries()]
    .sort((a, b) => a[1] - b[1])
    .map(([id]) => groupMap.get(id)!);
}

/**
 * Fallback user ID for dev/test environments where Clerk is not configured.
 * In production, authStore.internalUserId is always used instead.
 */
export const MVP_USER_ID = "00000000-0000-0000-0000-000000000001";

/** Returns the authenticated user's internal UUID, falling back to MVP seed UUID. */
function getUserId(): string {
  return useAuthStore.getState().internalUserId ?? MVP_USER_ID;
}

interface ResumeStore {
  // ─── State ─────────────────────────────────────────────────────────────────
  resumeId: string | null;
  /** Structured per-entry bullet groups — populated on generation complete or resume load. */
  entryGroups: EntryGroup[];
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
  /** Tracks the async generation job (FIX-08). */
  generationJobId: string | null;
  /** Current phase of the async generation pipeline (FIX-08). */
  generationStatus: GenerationStatus;

  // ─── Inline refinement ─────────────────────────────────────────────────────
  /** Pending refinements persisted to localStorage — survive navigation and refresh. */
  refinementQueue: RefinementQueueItem[];
  /** Bullet texts currently being refined (for per-card spinner). */
  refiningBullets: string[];

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
   * Full generation pipeline (FIX-08):
   * 1. POST /api/v1/resumes/generate → returns { job_id } immediately
   * 2. Persist job_id to project (fire-and-forget)
   * 3. Poll GET /api/v1/generation/jobs/:id/status every 3s until done/failed
   * 4. On done: fetch audit, trigger PDF render, start render polling, link resume
   */
  generate: (projectId?: string) => Promise<void>;
  /**
   * Polls GET /api/v1/generation/jobs/:id/status every 3 seconds.
   * On done: populates entryGroups, triggers render pipeline.
   * On failed: surfaces error.
   */
  pollGenerationStatus: () => void;
  /**
   * Standalone fit analysis (two-step JD workflow).
   * Calls POST /api/v1/resumes/fit-score and updates fitReport, fitScoreCacheHit, hashes.
   * pass forceRefresh=true to bypass server-side cache.
   */
  analyzeFit: (forceRefresh?: boolean) => Promise<void>;
  /** Polls the render job status every 2 seconds until done or failed. */
  pollRenderStatus: () => void;
  /**
   * Loads a saved resume from the database by ID.
   * Populates entryGroups and resumeId. Called on page load when a project has
   * a current_resume_id, so bullets survive page refresh.
   */
  loadResume: (resumeId: string) => Promise<void>;
  /**
   * Re-triggers a PDF render for the current resumeId.
   * Resets renderStatus to "queued" and starts polling.
   */
  rerender: () => Promise<void>;
  clearError: () => void;

  // ─── Inline refinement actions ─────────────────────────────────────────────
  /** Restores the queue from localStorage for a given project on page mount. */
  hydrateQueue: (projectId: string) => void;
  /** Fires an immediate single-bullet refinement → updates store → re-renders PDF. */
  refineBullet: (bulletText: string, sourceEntryId: string, section: string, instruction: string) => Promise<void>;
  /** Adds a refinement to the queue and persists to localStorage. */
  queueRefinement: (bulletText: string, sourceEntryId: string, section: string, instruction: string) => void;
  /** Fires all queued refinements in parallel → patches bullets → single re-render. */
  applyQueue: () => Promise<void>;
  /** Clears the full queue and removes from localStorage. */
  clearQueue: () => void;
  /** Removes a single item from the queue by bulletText. */
  removeFromQueue: (bulletText: string) => void;
}

export const useResumeStore = create<ResumeStore>((set, get) => ({
  resumeId: null,
  entryGroups: [],
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
  generationJobId: null,
  generationStatus: "idle",
  refinementQueue: [],
  refiningBullets: [],

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
    entryGroups: [],
    resumeId: null,
    auditManifest: null,
    renderJobId: null,
    renderStatus: "idle",
    isGenerating: false,
    error: null,
    generationJobId: null,
    generationStatus: "idle",
    refinementQueue: [],
    refiningBullets: [],
  }),

  invalidateFitScore: () => set({ contextChangedSinceAnalysis: true }),

  autoLoadCachedFitScore: async (jdText) => {
    if (!jdText.trim()) return;
    try {
      const resp = await api.getCachedFitScore(getUserId(), jdText);
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
      const resp = await api.analyzeFit(getUserId(), jdText, forceRefresh);
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
    console.log("[store] generate() start — enqueueing async job");

    if (projectId !== undefined) set({ currentProjectId: projectId });

    set({
      isGenerating: true,
      error: null,
      generationStatus: "queued",
      generationJobId: null,
      renderStatus: "idle",
      renderJobId: null,
      auditManifest: null,
      entryGroups: [],
    });

    try {
      // Step 1: Enqueue — returns immediately with { job_id, status: "queued" }
      const { job_id } = await api.generateResume(getUserId(), jdText);
      console.log("[store] generation job enqueued", { job_id });
      set({ generationJobId: job_id });

      // Step 2: Persist job_id to project so page-reload can resume polling (fire-and-forget)
      const pid = get().currentProjectId;
      if (pid) {
        api.updateProject(pid, { generation_job_id: job_id }).catch((err) => {
          console.warn("Failed to persist generation_job_id to project (non-fatal):", err);
        });
      }

      // Step 3: Start polling
      get().pollGenerationStatus();
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : "Generation failed",
        generationStatus: "failed",
        isGenerating: false,
      });
    }
  },

  pollGenerationStatus: () => {
    const poll = async () => {
      const { generationJobId, generationStatus } = get();

      // Stop if no job or already terminal
      if (!generationJobId || generationStatus === "done" || generationStatus === "failed") {
        return;
      }

      try {
        const status = await api.getGenerationStatus(generationJobId);
        console.log("[store] generation poll tick", { generationJobId, status: status.status });

        set({ generationStatus: status.status as GenerationStatus });

        if (status.status === "done") {
          const resumeId = status.resume_id ?? null;
          set({
            resumeId,
            entryGroups: status.entry_groups ?? [],
            fitReport: status.fit_report ?? null,
            fitScoreCacheHit: false,
            lastJdHash: null,
            lastContextHash: null,
            isGenerating: false,
          });
          console.log("[store] generation done", {
            resumeId,
            entryGroupCount: (status.entry_groups ?? []).length,
          });

          // Fetch audit manifest (non-blocking)
          if (resumeId) {
            api.getAuditManifest(resumeId).then((audit) => {
              set({ auditManifest: audit });
            }).catch(() => {
              // Audit manifest fetch failure is non-fatal — silently ignored
            });

            // Trigger PDF render
            try {
              const renderResp = await api.triggerRender(resumeId);
              console.log("[store] render triggered after generation", { job_id: renderResp.job_id });
              set({ renderJobId: renderResp.job_id, renderStatus: "queued" });
              get().pollRenderStatus();
            } catch (renderErr) {
              console.warn("Failed to trigger render after generation (non-fatal):", renderErr);
              set({ renderStatus: "failed", error: renderErr instanceof Error ? renderErr.message : "Render trigger failed" });
            }

            // Link resume to project (fire-and-forget)
            const pid = get().currentProjectId;
            if (pid) {
              api.updateProject(pid, { current_resume_id: resumeId }).catch((err) => {
                console.warn("Failed to link resume to project (non-fatal):", err);
              });
            }
          }

          return; // stop polling
        }

        if (status.status === "failed") {
          set({
            error: status.error ?? "Generation failed",
            isGenerating: false,
          });
          return; // stop polling
        }

        // queued | processing — continue polling every 3 seconds
        setTimeout(poll, 3000);
      } catch (e) {
        set({
          error: e instanceof Error ? e.message : "Failed to check generation status",
          generationStatus: "failed",
          isGenerating: false,
        });
      }
    };

    // Initial poll after a short delay to allow the worker to pick up the job
    setTimeout(poll, 2000);
  },

  rerender: async () => {
    const { resumeId } = get();
    if (!resumeId) return;
    set({ renderStatus: "queued", renderJobId: null });
    try {
      const renderResp = await api.triggerRender(resumeId);
      set({ renderJobId: renderResp.job_id, renderStatus: "queued" });
      get().pollRenderStatus();
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : "Re-render failed",
        renderStatus: "failed",
      });
    }
  },

  loadResume: async (resumeId) => {
    try {
      // Fetch bullets and latest render job in parallel
      const [detail, renderJob] = await Promise.all([
        api.getResume(resumeId),
        api.getResumeRenderJob(resumeId),
      ]);

      // Only restore states that are actionable right now.
      // "failed" is a historical record — do not replay it as the current session state.
      let restoredJobId: string | null = null;
      let restoredStatus: RenderStatus = "idle";

      if (renderJob) {
        if (renderJob.status === "done") {
          restoredStatus = "done";
          restoredJobId = renderJob.job_id;
        } else if (renderJob.status === "processing" || renderJob.status === "queued") {
          restoredStatus = renderJob.status === "processing" ? "rendering" : "queued";
          restoredJobId = renderJob.job_id;
        }
        // "failed" / any other status → stays "idle"
      }

      set({
        resumeId: detail.resume.id,
        // Use stored entry_groups (full typed display headers) when available (post-migration 014).
        // Fall back to bulletRowsToEntryGroups for legacy resumes — shows entry_header LaTeX as label.
        entryGroups: detail.entry_groups ?? bulletRowsToEntryGroups(detail.bullets),
        renderJobId: restoredJobId,
        renderStatus: restoredStatus,
      });

      // Resume polling if a render job was in-flight when the page was last closed
      if (restoredStatus === "rendering" || restoredStatus === "queued") {
        get().pollRenderStatus();
      }
    } catch {
      // Non-fatal — if the fetch fails, entryGroups stay empty (user can regenerate)
    }
  },

  // ─── Inline refinement ─────────────────────────────────────────────────────

  hydrateQueue: (projectId) => {
    set({ refinementQueue: loadQueue(projectId) });
  },

  refineBullet: async (bulletText, sourceEntryId, section, instruction) => {
    const { resumeId, entryGroups, currentProjectId } = get();
    if (!resumeId) return;

    // Mark bullet as in-flight
    set((s) => ({ refiningBullets: [...s.refiningBullets, bulletText] }));
    try {
      const result = await api.refineBullet(resumeId, {
        bullet_text: bulletText,
        source_entry_id: sourceEntryId,
        section,
        instruction,
      });

      if (!result.was_rejected) {
        // Patch the bullet in entryGroups
        const newGroups = entryGroups.map((g) => {
          if (g.source_entry_id !== sourceEntryId) return g;
          return {
            ...g,
            bullets: g.bullets.map((b) =>
              b.text === bulletText
                ? { ...b, text: result.refined_text, verified_line_count: result.verified_line_count as 1 | 2, was_adjusted: true }
                : b
            ),
          };
        });
        set({ entryGroups: newGroups });
        get().rerender();
      }

      // Remove from queue if it was queued
      if (currentProjectId) {
        const newQ = get().refinementQueue.filter((r) => r.bulletText !== bulletText);
        set({ refinementQueue: newQ });
        persistQueue(currentProjectId, newQ);
      }

      return result.was_rejected
        ? Promise.reject(new Error(result.rejection_reason ?? "Refinement rejected"))
        : Promise.resolve();
    } catch (e) {
      throw e;
    } finally {
      set((s) => ({ refiningBullets: s.refiningBullets.filter((t) => t !== bulletText) }));
    }
  },

  queueRefinement: (bulletText, sourceEntryId, section, instruction) => {
    const { currentProjectId, refinementQueue } = get();
    // Replace if already queued for this bullet
    const existing = refinementQueue.findIndex((r) => r.bulletText === bulletText);
    const item: RefinementQueueItem = { bulletText, sourceEntryId, section, instruction, originalText: bulletText };
    const newQ = existing >= 0
      ? refinementQueue.map((r, i) => (i === existing ? item : r))
      : [...refinementQueue, item];
    set({ refinementQueue: newQ });
    if (currentProjectId) persistQueue(currentProjectId, newQ);
  },

  applyQueue: async () => {
    const { refinementQueue, resumeId, currentProjectId } = get();
    if (!resumeId || refinementQueue.length === 0) return;

    // Fire all in parallel — each updates entryGroups on its own
    await Promise.allSettled(
      refinementQueue.map((item) =>
        get().refineBullet(item.bulletText, item.sourceEntryId, item.section, item.instruction)
      )
    );

    // Clear queue
    set({ refinementQueue: [] });
    if (currentProjectId) persistQueue(currentProjectId, []);

    // Single re-render after all settle (refineBullet also calls rerender, but those
    // may interleave — one final call ensures the last state is rendered)
    get().rerender();
  },

  clearQueue: () => {
    const { currentProjectId } = get();
    set({ refinementQueue: [] });
    if (currentProjectId) persistQueue(currentProjectId, []);
  },

  removeFromQueue: (bulletText) => {
    const { currentProjectId } = get();
    const newQ = get().refinementQueue.filter((r) => r.bulletText !== bulletText);
    set({ refinementQueue: newQ });
    if (currentProjectId) persistQueue(currentProjectId, newQ);
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
        const rawStatus = statusResp.status === "processing" ? "rendering" : statusResp.status;
        const newStatus = rawStatus as RenderStatus;
        console.log("[store] render poll tick", { renderJobId, rawStatus: statusResp.status, mappedStatus: newStatus });

        if (newStatus === "failed") {
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
          setTimeout(poll, 2000);
        }
      } catch {
        set({
          renderStatus: "failed",
          error: "Lost connection while waiting for render. Please try again.",
        });
      }
    };

    setTimeout(poll, 2000);
  },
}));
