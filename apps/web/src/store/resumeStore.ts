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

  // ─── Actions ───────────────────────────────────────────────────────────────
  setJdText: (text: string) => void;
  /**
   * Full generation pipeline:
   * 1. POST /api/v1/resumes/generate
   * 2. GET  /api/v1/resumes/:id/audit  (non-blocking)
   * 3. POST /api/v1/render
   * 4. Start polling render status
   */
  generate: () => Promise<void>;
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

  setJdText: (text) => set({ jdText: text }),
  clearError: () => set({ error: null }),

  generate: async () => {
    const { jdText } = get();
    if (!jdText.trim()) return;

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
      set({
        resumeId: generated.resume_id,
        bullets: generated.bullets,
        fitReport: generated.fit_report,
      });

      // Step 2: Fetch audit manifest (non-blocking — failure should not abort flow)
      api.getAuditManifest(generated.resume_id).then((audit) => {
        set({ auditManifest: audit });
      }).catch(() => {
        // Audit manifest fetch failure is non-fatal — silently ignored
      });

      // Step 3: Trigger PDF render
      const renderResp = await api.triggerRender(generated.resume_id);
      set({ renderJobId: renderResp.job_id, renderStatus: "queued" });

      // Step 4: Start polling
      get().pollRenderStatus();
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
        const { status } = await api.getRenderStatus(renderJobId);
        set({ renderStatus: status as RenderStatus });

        if (status !== "done" && status !== "failed") {
          // Continue polling every 2 seconds
          setTimeout(poll, 2000);
        }
      } catch {
        // Network error during poll — mark as failed
        set({ renderStatus: "failed" });
      }
    };

    // Initial poll (starts the loop)
    setTimeout(poll, 2000);
  },
}));
