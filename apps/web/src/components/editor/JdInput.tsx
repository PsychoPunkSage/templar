"use client";

import { Textarea } from "@/components/ui/textarea";
import { useResumeStore } from "@/store/resumeStore";

/**
 * Job description text input.
 * Uses controlled input pattern via Zustand store.
 */
export function JdInput() {
  const { jdText, setJdText } = useResumeStore();

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
        onChange={(e) => setJdText(e.target.value)}
        placeholder="Paste the job description here..."
        className="h-48 resize-none font-mono text-sm"
      />
    </div>
  );
}
