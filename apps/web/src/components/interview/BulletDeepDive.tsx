"use client";

import type { PrepQuestion, QuestionType } from "@templar/types";

interface BulletDeepDiveProps {
  questions: PrepQuestion[];
  activeFilter: "all" | QuestionType | "gap";
}

const questionTypeBadge: Record<QuestionType, string> = {
  behavioral: "bg-blue-100 dark:bg-blue-900/40 text-blue-700 dark:text-blue-300",
  technical:  "bg-violet-100 dark:bg-violet-900/40 text-violet-700 dark:text-violet-300",
  role_fit:   "bg-emerald-100 dark:bg-emerald-900/40 text-emerald-700 dark:text-emerald-300",
};

const questionTypeLabel: Record<QuestionType, string> = {
  behavioral: "Behavioral",
  technical:  "Technical",
  role_fit:   "Role fit",
};

export function BulletDeepDive({ questions, activeFilter }: BulletDeepDiveProps) {
  const visible = questions.filter(
    (q) =>
      activeFilter === "all" ||
      activeFilter === "gap" ||
      q.type === activeFilter,
  );

  if (visible.length === 0) {
    return (
      <p className="text-sm text-muted-foreground italic">
        No questions for this filter. Select &quot;All&quot; or another category.
      </p>
    );
  }

  return (
    <div className="space-y-3">
      {visible.map((q, i) => (
        <div
          key={i}
          className="rounded-md border border-border bg-card px-3 py-2.5 space-y-1.5"
        >
          <p className="text-sm text-foreground leading-relaxed">{q.text}</p>
          <span
            className={`inline-block text-xs font-medium px-2 py-0.5 rounded-full ${questionTypeBadge[q.type]}`}
          >
            {questionTypeLabel[q.type]}
          </span>
        </div>
      ))}
    </div>
  );
}
