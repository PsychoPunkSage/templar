"use client";

/**
 * AuthSync — mounts once in the root layout, invisible, zero markup.
 *
 * Responsibilities:
 *  1. Registers Clerk's getToken() as the api module's token getter so every
 *     apiFetch() call automatically includes a fresh Authorization header.
 *  2. On sign-in: calls GET /api/v1/auth/me to resolve the internal UUID
 *     (Clerk user ID → DB UUID), then stores it in authStore.
 *  3. On sign-out: clears the internal UUID from authStore.
 */

import { useEffect, useRef } from "react";
import { useAuth } from "@clerk/nextjs";
import { setTokenGetter } from "@/lib/api";
import { useAuthStore } from "@/store/authStore";

export function AuthSync() {
  const { userId, getToken, isSignedIn, isLoaded } = useAuth();
  const prevUserIdRef = useRef<string | null | undefined>(undefined);

  // Step 1: register the token getter once (getToken reference is stable across renders)
  useEffect(() => {
    setTokenGetter(async () => {
      try {
        return await getToken();
      } catch {
        return null;
      }
    });
  }, [getToken]);

  // Step 2: sync auth state whenever Clerk's loaded state changes
  useEffect(() => {
    if (!isLoaded) return;

    // Avoid redundant syncUser() calls when Clerk re-renders but userId hasn't changed
    if (userId === prevUserIdRef.current) return;
    prevUserIdRef.current = userId;

    if (isSignedIn && userId) {
      useAuthStore.getState().syncUser();
    } else {
      useAuthStore.getState().clearUser();
    }
  }, [isLoaded, isSignedIn, userId]);

  return null;
}
