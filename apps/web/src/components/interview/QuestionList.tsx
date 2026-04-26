"use client";

import type { PrepBullet, GapQuestion, QuestionType } from "@templar/types";

type FilterType = "all" | QuestionType | "gap";

interface QuestionListProps {
  bullets: PrepBullet[];
  gapQuestions: GapQuestion[];
  selectedHash: string | null;
  filter: FilterType;
  onSelectBullet: (hash: string | null) => void;
  onFilterChange: (filter: FilterType) => void;
}

const FILTER_TABS: { key: FilterType; label: string }[] = [
  { key: "all",        label: "All" },
  { key: "behavioral", label: "Behavioral" },
  { key: "technical",  label: "Technical" },
  { key: "gap",        label: "Gaps" },
];

export function QuestionList({
  bullets,
  gapQuestions,
  selectedHash,
  filter,
  onSelectBullet,
  onFilterChange,
}: QuestionListProps) {
  // Count questions per filter for the tab badge
  const totalForFilter = (f: FilterType): number => {
    if (f === "gap") return gapQuestions.length;
    if (f === "all") {
      return (
        bullets.reduce((acc, b) => acc + b.questions.length, 0) +
        gapQuestions.length
      );
    }
    return bullets.reduce(
      (acc, b) => acc + b.questions.filter((q) => q.type === f).length,
      0,
    );
  };

  // Bullets that have questions matching the filter
  const visibleBullets =
    filter === "gap"
      ? []
      : bullets.filter((b) =>
          filter === "all"
            ? b.questions.length > 0
            : b.questions.some((q) => q.type === filter),
        );

  return (
    <div className="flex flex-col h-full">
      {/* Filter tabs */}
      <div className="flex gap-1 px-1 pb-3 border-b border-border flex-wrap">
        {FILTER_TABS.map((tab) => {
          const count = totalForFilter(tab.key);
          const active = filter === tab.key;
          return (
            <button
              key={tab.key}
              onClick={() => onFilterChange(tab.key)}
              className={`px-3 py-1 rounded-full text-xs font-medium transition-colors ${
                active
                  ? "bg-indigo-600 text-white"
                  : "bg-muted text-muted-foreground hover:bg-muted/80"
              }`}
            >
              {tab.label}
              {count > 0 && (
                <span className={`ml-1.5 ${active ? "text-indigo-200" : "text-muted-foreground"}`}>
                  {count}
                </span>
              )}
            </button>
          );
        })}
      </div>

      <div className="flex-1 overflow-y-auto space-y-1 pt-3">
        {/* Gap questions section */}
        {(filter === "gap" || filter === "all") && gapQuestions.length > 0 && (
          <div className="mb-3">
            <p className="text-xs font-semibold text-muted-foreground uppercase tracking-wide mb-2 px-1">
              Gap questions
            </p>
            {gapQuestions.map((gq, i) => (
              <button
                key={`gap-${i}`}
                onClick={() => onSelectBullet(null)}
                className="w-full text-left px-3 py-2.5 rounded-md hover:bg-muted/60 transition-colors group mb-1"
              >
                <p className="text-sm text-foreground leading-snug line-clamp-2">{gq.text}</p>
                <span className="text-xs text-amber-600 dark:text-amber-400 mt-0.5 block">
                  Gap: {gq.gap_area}
                </span>
              </button>
            ))}
          </div>
        )}

        {/* Bullet-linked questions */}
        {filter !== "gap" && visibleBullets.length === 0 && (
          <p className="text-sm text-muted-foreground italic px-1 py-4 text-center">
            No questions in this category.
          </p>
        )}

        {visibleBullets.map((bullet) => {
          const qCount =
            filter === "all"
              ? bullet.questions.length
              : bullet.questions.filter((q) => q.type === filter).length;
          const isSelected = selectedHash === bullet.bullet_hash;

          return (
            <button
              key={bullet.bullet_hash}
              onClick={() =>
                onSelectBullet(isSelected ? null : bullet.bullet_hash)
              }
              className={`w-full text-left px-3 py-2.5 rounded-md transition-colors mb-1 ${
                isSelected
                  ? "bg-indigo-50 dark:bg-indigo-950/30 border border-indigo-200 dark:border-indigo-800"
                  : "hover:bg-muted/60"
              }`}
            >
              <p className="text-sm text-foreground leading-snug line-clamp-2">
                {bullet.bullet_text}
              </p>
              <p className="text-xs text-muted-foreground mt-0.5">
                {qCount} question{qCount !== 1 ? "s" : ""}
              </p>
            </button>
          );
        })}
      </div>
    </div>
  );
}
