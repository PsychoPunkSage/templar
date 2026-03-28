"use client";

import { useEffect, useRef } from "react";
import { Textarea } from "@/components/ui/textarea";
import { useResumeStore } from "@/store/resumeStore";
import { api } from "@/lib/api";

interface JdInputProps {
  /** The project ID to persist last_jd_text against on change. */
  projectId: string;
}

/**
 * Job description text input.
 * Uses controlled input pattern via Zustand store.
 *
 * Persists the JD text to the project's `last_jd_text` column with a 2-second
 * debounce so that the user's JD is automatically restored on next visit.
 * The save is fire-and-forget — failure is silently ignored.
 */
export function JdInput({ projectId }: JdInputProps) {
  const { jdText, setJdText } = useResumeStore();
  const persistDebounce = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Cleanup debounce timer on unmount to avoid memory leaks / stale calls.
  useEffect(() => {
    return () => {
      if (persistDebounce.current) clearTimeout(persistDebounce.current);
    };
  }, []);

  const handleChange = (value: string) => {
    setJdText(value);

    // Debounce: persist JD to project after 2 seconds of inactivity.
    if (persistDebounce.current) clearTimeout(persistDebounce.current);
    persistDebounce.current = setTimeout(() => {
      api.updateProject(projectId, { last_jd_text: value }).catch(() => {
        // Fire-and-forget — silently ignore network errors
      });
    }, 2000);
  };

  return (
    <div className="flex flex-col gap-2">
      <label
        htmlFor="jd-input"
        className="text-sm font-medium text-muted-foreground"
      >
        Job Description
      </label>
      <Textarea
        id="jd-input"
        value={jdText}
        onChange={(e) => handleChange(e.target.value)}
        placeholder="Paste the job description here..."
        className="h-48 resize-none font-mono text-sm"
      />
    </div>
  );
}
