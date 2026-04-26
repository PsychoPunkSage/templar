"use client";

import { create } from "zustand";
import { api } from "@/lib/api";
import { useAuthStore } from "@/store/authStore";

export type CoverLetterTone = "formal" | "conversational";
export type CoverLetterFocus = "technical" | "leadership" | "culture_fit";
export type CoverLetterStatus = "idle" | "generating" | "done" | "failed";

export interface CoverLetterParagraph {
  role: "hook" | "fit" | "culture" | "close";
  text: string;
}

interface CoverLetterStore {
  // ─── State ─────────────────────────────────────────────────────────────────
  coverId: string | null;
  content: CoverLetterParagraph[] | null;
  companyName: string | null;
  roleTitle: string | null;
  status: CoverLetterStatus;
  tone: CoverLetterTone;
  focus: CoverLetterFocus;
  /** The JD text that was active when the current CL was generated — for staleness detection. */
  generatedWithJdText: string | null;
  error: string | null;

  // ─── Actions ───────────────────────────────────────────────────────────────
  generate: (
    jdText: string,
    resumeId: string | null,
    personaId: string | null
  ) => Promise<void>;
  loadCoverLetter: (id: string) => Promise<void>;
  /** Auto-loads the most recent cover letter for a given resume. No-op if none exists. */
  loadCoverLetterForResume: (userId: string, resumeId: string) => Promise<void>;
  setTone: (tone: CoverLetterTone) => void;
  setFocus: (focus: CoverLetterFocus) => void;
  reset: () => void;
  clearError: () => void;
}

/** Fallback user ID for dev/test environments where Clerk is not configured. */
const MVP_USER_ID = "00000000-0000-0000-0000-000000000001";

function getUserId(): string {
  return useAuthStore.getState().internalUserId ?? MVP_USER_ID;
}

const initialState = {
  coverId: null,
  content: null,
  companyName: null,
  roleTitle: null,
  status: "idle" as CoverLetterStatus,
  tone: "formal" as CoverLetterTone,
  focus: "technical" as CoverLetterFocus,
  generatedWithJdText: null,
  error: null,
};

export const useCoverLetterStore = create<CoverLetterStore>((set) => ({
  ...initialState,

  setTone: (tone) => set({ tone }),
  setFocus: (focus) => set({ focus }),
  clearError: () => set({ error: null }),

  reset: () => set(initialState),

  generate: async (jdText, resumeId, personaId) => {
    if (!jdText.trim()) return;
    set({ status: "generating", error: null });

    try {
      const userId = getUserId();
      const result = await api.generateCoverLetter(
        userId,
        jdText,
        useCoverLetterStore.getState().tone,
        useCoverLetterStore.getState().focus,
        resumeId,
        personaId
      );

      set({
        coverId: result.id,
        content: result.content as CoverLetterParagraph[],
        companyName: result.company_name ?? null,
        roleTitle: result.role_title ?? null,
        status: "done",
        generatedWithJdText: jdText,
      });
    } catch (e) {
      set({
        status: "failed",
        error: e instanceof Error ? e.message : "Cover letter generation failed",
      });
    }
  },

  loadCoverLetter: async (id) => {
    try {
      const result = await api.getCoverLetter(id);
      set({
        coverId: result.id,
        content: result.content as CoverLetterParagraph[],
        companyName: result.company_name ?? null,
        roleTitle: result.role_title ?? null,
        status: "done",
      });
    } catch {
      // Non-fatal — if load fails, pane stays in idle state
    }
  },

  loadCoverLetterForResume: async (userId, resumeId) => {
    try {
      const resp = await api.listCoverLetters(userId, resumeId);
      if (resp.cover_letters.length === 0) return;
      // listCoverLetters returns newest-first (ORDER BY created_at DESC)
      const latest = resp.cover_letters[0];
      set({
        coverId: latest.id,
        content: latest.content as CoverLetterParagraph[],
        companyName: latest.company_name ?? null,
        roleTitle: latest.role_title ?? null,
        status: "done",
        // generatedWithJdText stays null — original JD text not available here;
        // staleness won't fire until the user edits the JD in this session.
      });
    } catch {
      // Non-fatal — pane stays in idle/generation form state
    }
  },
}));
