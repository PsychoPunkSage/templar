"use client";

import { useState } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import { useResumeStore } from "@/store/resumeStore";
import { Button } from "@/components/ui/button";
import type { FitMatch, Gap } from "@templar/types";

type Filter = "all" | "strong" | "partial" | "gaps";

function StrengthDots({ strength }: { strength: number }) {
  const filled = Math.round(strength * 5);
  return (
    <span className="flex gap-0.5 items-center">
      {Array.from({ length: 5 }).map((_, i) => (
        <span
          key={i}
          className={`inline-block h-1.5 w-1.5 rounded-full ${
            i < filled ? "bg-current opacity-80" : "bg-muted-foreground/25"
          }`}
        />
      ))}
    </span>
  );
}

function MatchRow({ match, color }: { match: FitMatch; color: string }) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div
      className="py-1.5 border-b border-border/40 last:border-0 cursor-pointer select-none"
      onClick={() => setExpanded((e) => !e)}
    >
      {/* Header row — always visible */}
      <div className={`flex items-center justify-between ${color}`}>
        <span className="text-xs font-medium">{match.dimension}</span>
        <div className="flex items-center gap-1.5">
          <StrengthDots strength={match.strength} />
          <span className="text-xs tabular-nums opacity-70">
            {Math.round(match.strength * 100)}%
          </span>
          {expanded
            ? <ChevronUp className="h-3 w-3 opacity-50" />
            : <ChevronDown className="h-3 w-3 opacity-50" />}
        </div>
      </div>

      {/* Evidence — collapsed by default */}
      {expanded && (
        <div className="mt-1.5 flex flex-col gap-1">
          {match.context_evidence && (
            <p className="text-xs text-muted-foreground">
              <span className="font-medium text-foreground/70">You:</span>{" "}
              {match.context_evidence}
            </p>
          )}
          {match.jd_requirement && (
            <p className="text-xs text-muted-foreground">
              <span className="font-medium text-foreground/70">JD:</span>{" "}
              {match.jd_requirement}
            </p>
          )}
        </div>
      )}
    </div>
  );
}

function GapRow({ gap }: { gap: Gap }) {
  return (
    <div className="flex items-center justify-between py-1.5 border-b border-border/40 last:border-0 text-destructive">
      <span className="text-xs font-medium">{gap.keyword}</span>
      {gap.jd_frequency > 0 && (
        <span className="text-xs opacity-70 tabular-nums">{gap.jd_frequency}× in JD</span>
      )}
    </div>
  );
}

export function FitReportPanel() {
  const {
    fitReport,
    fitScoreCacheHit,
    fitScoreLoading,
    analyzeFit,
    contextChangedSinceAnalysis,
  } = useResumeStore();

  const [filter, setFilter] = useState<Filter>("all");

  if (!fitReport) return null;

  const score = fitReport.overall_score;
  const scoreColor =
    score >= 70 ? "text-green-600" : score >= 50 ? "text-yellow-600" : "text-destructive";
  const barColor =
    score >= 70 ? "bg-green-500" : score >= 50 ? "bg-yellow-500" : "bg-destructive";

  const filters: { key: Filter; label: string; count?: number; activeClass: string }[] = [
    { key: "all",     label: "All",                                    activeClass: "bg-muted text-foreground" },
    { key: "strong",  label: `✓ Strong (${fitReport.strong_matches.length})`,  activeClass: "bg-green-100 text-green-700 dark:bg-green-950/40 dark:text-green-400" },
    { key: "partial", label: `~ Partial (${fitReport.partial_matches.length})`, activeClass: "bg-yellow-100 text-yellow-700 dark:bg-yellow-950/40 dark:text-yellow-400" },
    { key: "gaps",    label: `✗ Gaps (${fitReport.gaps.length})`,              activeClass: "bg-red-100 text-destructive dark:bg-red-950/30" },
  ];

  const showStrong  = filter === "all" || filter === "strong";
  const showPartial = filter === "all" || filter === "partial";
  const showGaps    = filter === "all" || filter === "gaps";

  return (
    <div className="border rounded-lg p-3 flex flex-col gap-3">
      {/* Stale score banner */}
      {contextChangedSinceAnalysis && (
        <div className="flex items-center justify-between text-xs text-yellow-600 bg-yellow-50 dark:bg-yellow-950/20 rounded px-2 py-1">
          <span>Context updated — score may be stale.</span>
          <Button
            variant="ghost"
            size="sm"
            className="h-6 px-2 text-xs"
            onClick={() => analyzeFit(false)}
            disabled={fitScoreLoading}
          >
            Re-analyze
          </Button>
        </div>
      )}

      {/* Score section */}
      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium">Fit Score</span>
            <span className="text-xs text-muted-foreground">via {fitReport.scorer_backend}</span>
            {fitScoreCacheHit === true && (
              <span className="text-xs text-muted-foreground border border-border/60 rounded px-1.5 py-0.5 leading-none">
                cached
              </span>
            )}
            {fitScoreCacheHit === false && (
              <span className="text-xs text-muted-foreground border border-border/60 rounded px-1.5 py-0.5 leading-none">
                live
              </span>
            )}
          </div>
          <span className={`text-3xl font-bold tabular-nums leading-none ${scoreColor}`}>
            {score}
            <span className="text-sm font-normal text-muted-foreground ml-0.5">/100</span>
          </span>
        </div>

        {/* Progress bar */}
        <div className="h-1.5 w-full rounded-full bg-muted overflow-hidden">
          <div
            className={`h-full rounded-full transition-all duration-500 ${barColor}`}
            style={{ width: `${score}%` }}
          />
        </div>
      </div>

      {/* Recommendation */}
      {fitReport.recommendation && (
        <p className="text-xs text-muted-foreground italic border-l-2 border-border pl-2">
          {fitReport.recommendation}
        </p>
      )}

      {/* Filter pills */}
      <div className="flex flex-wrap gap-1.5">
        {filters.map((f) => (
          <button
            key={f.key}
            onClick={() => setFilter(f.key)}
            className={`text-xs rounded-full px-2.5 py-1 font-medium transition-colors ${
              filter === f.key
                ? f.activeClass
                : "text-muted-foreground hover:text-foreground hover:bg-muted/60"
            }`}
          >
            {f.label}
          </button>
        ))}
      </div>

      {/* Strong matches */}
      {showStrong && fitReport.strong_matches.length > 0 && (
        <div>
          {filter === "all" && (
            <p className="text-xs font-semibold mb-1 text-muted-foreground uppercase tracking-wider">
              Strong Matches
            </p>
          )}
          <div className="flex flex-col">
            {fitReport.strong_matches.slice(0, 8).map((m: FitMatch) => (
              <MatchRow key={m.dimension} match={m} color="text-green-700 dark:text-green-500" />
            ))}
          </div>
        </div>
      )}

      {/* Partial matches */}
      {showPartial && fitReport.partial_matches.length > 0 && (
        <div>
          {filter === "all" && (
            <p className="text-xs font-semibold mb-1 text-muted-foreground uppercase tracking-wider">
              Partial Matches
            </p>
          )}
          <div className="flex flex-col">
            {fitReport.partial_matches.slice(0, 6).map((m: FitMatch) => (
              <MatchRow key={m.dimension} match={m} color="text-yellow-700 dark:text-yellow-500" />
            ))}
          </div>
        </div>
      )}

      {/* Gaps */}
      {showGaps && fitReport.gaps.length > 0 && (
        <div>
          {filter === "all" && (
            <p className="text-xs font-semibold mb-1 text-muted-foreground uppercase tracking-wider">
              Gaps
            </p>
          )}
          <div className="flex flex-col">
            {fitReport.gaps.slice(0, 5).map((g: Gap) => (
              <GapRow key={g.keyword} gap={g} />
            ))}
          </div>
        </div>
      )}

      {/* Cache footer */}
      {fitScoreCacheHit === true && (
        <div className="flex items-center justify-between pt-1 border-t border-border/50">
          <span className="text-xs text-muted-foreground">
            Score unchanged since last analysis.
          </span>
          <Button
            variant="outline"
            size="sm"
            className="text-xs h-7"
            onClick={() => analyzeFit(true)}
            disabled={fitScoreLoading}
          >
            {fitScoreLoading ? "Analyzing..." : "Recalculate"}
          </Button>
        </div>
      )}
    </div>
  );
}
