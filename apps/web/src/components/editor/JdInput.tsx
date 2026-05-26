"use client";

import { useEffect, useRef } from "react";
import { AnimatedAiInput } from "@/components/ui/animated-ai-input";
import { useResumeStore } from "@/store/resumeStore";
import { api } from "@/lib/api";

interface JdInputProps {
  projectId: string;
}

export function JdInput({ projectId }: JdInputProps) {
  const { jdText, setJdText } = useResumeStore();
  const persistDebounce = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (persistDebounce.current) clearTimeout(persistDebounce.current);
    };
  }, []);

  const handleChange = (value: string) => {
    setJdText(value);
    if (persistDebounce.current) clearTimeout(persistDebounce.current);
    persistDebounce.current = setTimeout(() => {
      api.updateProject(projectId, { last_jd_text: value }).catch(() => {});
    }, 2000);
  };

  return (
    <AnimatedAiInput
      value={jdText}
      onChange={handleChange}
      placeholder="Paste the job description here…"
      minRows={6}
      maxRows={16}
    />
  );
}
