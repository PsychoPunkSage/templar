"use client";

import { create } from "zustand";
import { api } from "@/lib/api";

interface AuthStore {
  /** Internal UUID from our DB (used for all API calls). null while loading. */
  internalUserId: string | null;
  /** True once we've attempted to resolve the internal user ID. */
  isAuthReady: boolean;
  /**
   * Called by AuthSync when Clerk reports a signed-in user.
   * Calls GET /api/v1/auth/me to resolve the internal UUID, then stores it.
   */
  syncUser: () => Promise<void>;
  /** Called by AuthSync when Clerk reports sign-out. */
  clearUser: () => void;
}

export const useAuthStore = create<AuthStore>((set) => ({
  internalUserId: null,
  isAuthReady: false,

  syncUser: async () => {
    try {
      const { user_id } = await api.authMe();
      set({ internalUserId: user_id, isAuthReady: true });
    } catch (e) {
      console.warn("[authStore] Failed to resolve internal user ID:", e);
      set({ internalUserId: null, isAuthReady: true });
    }
  },

  clearUser: () => set({ internalUserId: null, isAuthReady: true }),
}));
