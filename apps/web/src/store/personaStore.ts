"use client";

import { create } from "zustand";
import { api } from "@/lib/api";
import { MVP_USER_ID } from "@/store/resumeStore";
import { useAuthStore } from "@/store/authStore";
import type { Persona } from "@templar/types";

function getUserId(): string {
  return useAuthStore.getState().internalUserId ?? MVP_USER_ID;
}

interface PersonaStore {
  personas: Persona[];
  isLoading: boolean;
  isSaving: boolean;
  error: string | null;

  loadPersonas: () => Promise<void>;
  createPersona: (
    name: string,
    emphasizedTags: string[],
    suppressedTags: string[],
    tonePreference?: string,
  ) => Promise<Persona>;
  updatePersona: (
    id: string,
    data: Partial<Pick<Persona, "name" | "emphasized_tags" | "suppressed_tags" | "tone_preference">>,
  ) => Promise<void>;
  deletePersona: (id: string) => Promise<void>;
  clearError: () => void;
}

export const usePersonaStore = create<PersonaStore>((set) => ({
  personas: [],
  isLoading: false,
  isSaving: false,
  error: null,

  clearError: () => set({ error: null }),

  loadPersonas: async () => {
    set({ isLoading: true, error: null });
    try {
      const { personas } = await api.listPersonas(getUserId());
      set({ personas });
    } catch (e) {
      set({ error: e instanceof Error ? e.message : "Failed to load personas" });
    } finally {
      set({ isLoading: false });
    }
  },

  createPersona: async (name, emphasizedTags, suppressedTags, tonePreference) => {
    set({ isSaving: true, error: null });
    try {
      const persona = await api.createPersona({
        user_id: getUserId(),
        name,
        emphasized_tags: emphasizedTags,
        suppressed_tags: suppressedTags,
        tone_preference: tonePreference,
      });
      set((s) => ({ personas: [persona, ...s.personas] }));
      return persona;
    } catch (e) {
      const msg = e instanceof Error ? e.message : "Failed to create persona";
      set({ error: msg });
      throw e;
    } finally {
      set({ isSaving: false });
    }
  },

  updatePersona: async (id, data) => {
    set({ isSaving: true, error: null });
    try {
      const updated = await api.updatePersona(id, {
        name: data.name,
        emphasized_tags: data.emphasized_tags,
        suppressed_tags: data.suppressed_tags,
        tone_preference: data.tone_preference,
      });
      set((s) => ({
        personas: s.personas.map((p) => (p.id === id ? updated : p)),
      }));
    } catch (e) {
      set({ error: e instanceof Error ? e.message : "Failed to update persona" });
      throw e;
    } finally {
      set({ isSaving: false });
    }
  },

  deletePersona: async (id) => {
    try {
      await api.deletePersona(id);
      set((s) => ({ personas: s.personas.filter((p) => p.id !== id) }));
    } catch (e) {
      set({ error: e instanceof Error ? e.message : "Failed to delete persona" });
      throw e;
    }
  },
}));
