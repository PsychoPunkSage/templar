"use client";

import { create } from "zustand";
import { api } from "@/lib/api";
import type {
  PrepMeta,
  PrepBullet,
  PrepStatus,
  QuestionType,
  UpdateCompanyRequest,
} from "@templar/types";

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

type FilterType = "all" | QuestionType | "gap";

interface InterviewPrepState {
  projectId: string | null;
  status: PrepStatus | "idle";
  meta: PrepMeta | null;
  bullets: PrepBullet[];
  selectedBulletHash: string | null;
  questionFilter: FilterType;
  isTriggering: boolean;
  isPolling: boolean;
  error: string | null;
}

interface InterviewPrepActions {
  loadPrep: (projectId: string) => Promise<void>;
  triggerPrep: (projectId: string) => Promise<void>;
  pollStatus: (projectId: string) => void;
  stopPolling: () => void;
  updateCompany: (projectId: string, ctx: UpdateCompanyRequest) => Promise<void>;
  selectBullet: (hash: string | null) => void;
  setFilter: (filter: FilterType) => void;
  clearError: () => void;
  reset: () => void;
}

type InterviewPrepStore = InterviewPrepState & InterviewPrepActions;

// ─────────────────────────────────────────────────────────────────────────────
// Store
// ─────────────────────────────────────────────────────────────────────────────

const INITIAL: InterviewPrepState = {
  projectId: null,
  status: "idle",
  meta: null,
  bullets: [],
  selectedBulletHash: null,
  questionFilter: "all",
  isTriggering: false,
  isPolling: false,
  error: null,
};

let _pollIntervalId: ReturnType<typeof setInterval> | null = null;

export const useInterviewPrepStore = create<InterviewPrepStore>((set, get) => ({
  ...INITIAL,

  loadPrep: async (projectId: string) => {
    // Reset if switching projects
    if (get().projectId !== projectId) {
      get().reset();
    }
    set({ projectId });

    try {
      const data = await api.getInterviewPrep(projectId);
      const effectiveStatus: PrepStatus | "idle" = data.meta?.status ?? "idle";

      set({
        meta: data.meta,
        bullets: data.bullets,
        status: effectiveStatus,
        error: null,
      });

      // If generating, start polling
      if (effectiveStatus === "generating") {
        get().pollStatus(projectId);
      }
    } catch (e) {
      set({ error: e instanceof Error ? e.message : "Failed to load prep data" });
    }
  },

  triggerPrep: async (projectId: string) => {
    set({ isTriggering: true, error: null });
    try {
      await api.triggerInterviewPrep(projectId);
      set({ status: "generating", isTriggering: false });
      get().pollStatus(projectId);
    } catch (e) {
      set({
        isTriggering: false,
        error: e instanceof Error ? e.message : "Failed to start prep generation",
      });
    }
  },

  pollStatus: (projectId: string) => {
    // Clear any existing poll
    get().stopPolling();
    set({ isPolling: true });

    _pollIntervalId = setInterval(async () => {
      try {
        const statusData = await api.getInterviewPrepStatus(projectId);
        set({ status: statusData.status });

        if (statusData.status === "ready") {
          get().stopPolling();
          // Reload full data
          const data = await api.getInterviewPrep(projectId);
          set({
            meta: data.meta,
            bullets: data.bullets,
            status: "ready",
          });
        } else if (statusData.status === "failed") {
          get().stopPolling();
          set({ error: "Interview prep generation failed. Please try again." });
        }
      } catch (e) {
        // Silent polling error — keep retrying unless job is done
        console.warn("Interview prep poll error:", e);
      }
    }, 3000);
  },

  stopPolling: () => {
    if (_pollIntervalId !== null) {
      clearInterval(_pollIntervalId);
      _pollIntervalId = null;
    }
    set({ isPolling: false });
  },

  updateCompany: async (projectId: string, ctx: UpdateCompanyRequest) => {
    set({ error: null });
    try {
      const updatedMeta = await api.updateInterviewPrepCompany(projectId, ctx);
      if (updatedMeta) {
        set({ meta: updatedMeta });
      }
    } catch (e) {
      set({
        error: e instanceof Error ? e.message : "Failed to update company context",
      });
    }
  },

  selectBullet: (hash: string | null) => {
    set({ selectedBulletHash: hash });
  },

  setFilter: (filter: FilterType) => {
    set({ questionFilter: filter });
  },

  clearError: () => set({ error: null }),

  reset: () => {
    get().stopPolling();
    set({ ...INITIAL });
  },
}));
