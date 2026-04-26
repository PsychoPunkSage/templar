"use client";

import { AlertTriangle, Clock, Loader2, RefreshCw } from "lucide-react";
import type { PrepStatus } from "@templar/types";

interface PrepStatusBannerProps {
  status: PrepStatus | "idle";
  isStale: boolean;
  expiresAt?: string;
  onTrigger: () => void;
  isTriggering: boolean;
}

export function PrepStatusBanner({
  status,
  isStale,
  expiresAt,
  onTrigger,
  isTriggering,
}: PrepStatusBannerProps) {
  if (status === "generating") {
    return (
      <div className="flex items-center gap-2 px-4 py-2.5 bg-indigo-50 dark:bg-indigo-950/30 border-b border-indigo-200 dark:border-indigo-800 text-sm text-indigo-700 dark:text-indigo-300">
        <Loader2 className="h-4 w-4 animate-spin shrink-0" />
        <span>Updating interview prep... This may take a minute.</span>
      </div>
    );
  }

  if (status === "failed") {
    return (
      <div className="flex items-center justify-between gap-2 px-4 py-2.5 bg-destructive/10 border-b border-destructive/20 text-sm text-destructive">
        <div className="flex items-center gap-2">
          <AlertTriangle className="h-4 w-4 shrink-0" />
          <span>Interview prep generation failed.</span>
        </div>
        <button
          onClick={onTrigger}
          disabled={isTriggering}
          className="text-xs underline hover:no-underline disabled:opacity-50"
        >
          {isTriggering ? "Retrying..." : "Retry"}
        </button>
      </div>
    );
  }

  if (isStale && status === "ready") {
    return (
      <div className="flex items-center justify-between gap-2 px-4 py-2.5 bg-amber-50 dark:bg-amber-950/30 border-b border-amber-200 dark:border-amber-800 text-sm text-amber-700 dark:text-amber-400">
        <div className="flex items-center gap-2">
          <AlertTriangle className="h-4 w-4 shrink-0" />
          <span>Your resume has changed. Interview prep may be out of date.</span>
        </div>
        <button
          onClick={onTrigger}
          disabled={isTriggering}
          className="flex items-center gap-1 text-xs underline hover:no-underline disabled:opacity-50"
        >
          <RefreshCw className="h-3 w-3" />
          {isTriggering ? "Updating..." : "Refresh prep"}
        </button>
      </div>
    );
  }

  if (expiresAt && status === "ready") {
    const daysLeft = Math.max(
      0,
      Math.ceil(
        (new Date(expiresAt).getTime() - Date.now()) / (1000 * 60 * 60 * 24),
      ),
    );
    if (daysLeft <= 7) {
      return (
        <div className="flex items-center gap-2 px-4 py-2.5 bg-muted/60 border-b text-sm text-muted-foreground">
          <Clock className="h-4 w-4 shrink-0" />
          <span>
            Interview prep expires in {daysLeft} day{daysLeft !== 1 ? "s" : ""}.
          </span>
          <button
            onClick={onTrigger}
            disabled={isTriggering}
            className="underline hover:no-underline disabled:opacity-50 text-xs ml-1"
          >
            Refresh now
          </button>
        </div>
      );
    }
  }

  return null;
}
