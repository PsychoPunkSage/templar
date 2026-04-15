"use client";

import { create } from "zustand";
import { api } from "@/lib/api";
import { MVP_USER_ID } from "@/store/resumeStore";
import { useAuthStore } from "@/store/authStore";
import type { UserProfileResponse, UpsertProfileRequest } from "@templar/types";

function getUserId(): string {
  return useAuthStore.getState().internalUserId ?? MVP_USER_ID;
}

interface ProfileStore {
  profile: UserProfileResponse | null;
  isLoading: boolean;
  isSaving: boolean;
  error: string | null;
  loadProfile: () => Promise<void>;
  saveProfile: (data: Omit<UpsertProfileRequest, "user_id">) => Promise<void>;
  clearError: () => void;
}

export const useProfileStore = create<ProfileStore>((set) => ({
  profile: null,
  isLoading: false,
  isSaving: false,
  error: null,

  clearError: () => set({ error: null }),

  loadProfile: async () => {
    set({ isLoading: true, error: null });
    try {
      const profile = await api.getProfile(getUserId());
      set({ profile });
    } catch (e) {
      set({ error: e instanceof Error ? e.message : "Failed to load profile" });
    } finally {
      set({ isLoading: false });
    }
  },

  saveProfile: async (data) => {
    set({ isSaving: true, error: null });
    try {
      const updated = await api.updateProfile({ user_id: getUserId(), ...data });
      set({ profile: updated });
    } catch (e) {
      set({ error: e instanceof Error ? e.message : "Failed to save profile" });
    } finally {
      set({ isSaving: false });
    }
  },
}));
