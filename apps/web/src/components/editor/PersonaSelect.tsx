"use client";

import { useEffect } from "react";
import { usePersonaStore } from "@/store/personaStore";
import { useAuthStore } from "@/store/authStore";
import { MVP_USER_ID } from "@/store/resumeStore";

interface PersonaSelectProps {
  value: string | null;
  onChange: (personaId: string | null) => void;
}

export function PersonaSelect({ value, onChange }: PersonaSelectProps) {
  const { personas, loadPersonas } = usePersonaStore();
  const userId = useAuthStore.getState().internalUserId ?? MVP_USER_ID;

  useEffect(() => {
    loadPersonas();
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [userId]);

  const selectedPersona = personas.find((p) => p.id === value) ?? null;

  return (
    <div className="flex items-center gap-2">
      <select
        value={value ?? ""}
        onChange={(e) => onChange(e.target.value || null)}
        className="h-8 rounded-md border border-input bg-background px-2 text-xs focus:outline-none focus:ring-2 focus:ring-ring text-foreground"
      >
        <option value="">No persona</option>
        {personas.map((p) => (
          <option key={p.id} value={p.id}>
            {p.name}
          </option>
        ))}
      </select>
      {selectedPersona && (
        <span className="shrink-0 rounded-full bg-indigo-100 dark:bg-indigo-900/40 px-2 py-0.5 text-xs text-indigo-700 dark:text-indigo-300 font-medium">
          {selectedPersona.name}
        </span>
      )}
    </div>
  );
}
