"use client";

import { useResumeStore } from "@/store/resumeStore";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { FitMatch, Gap } from "@templar/types";

/**
 * Displays the fit scoring report: overall score, matched requirements,
 * and gaps between user context and the job description.
 *
 * Uses the actual FitReport shape from the Rust backend:
 * { overall_score, strong_matches, partial_matches, gaps, recommendation, scorer_backend }
 *
 * Shows a cache badge (cached / live) next to the score header.
 * When the score is from cache, offers a "Recalculate" button to force a fresh LLM score.
 */
export function FitReportPanel() {
  const {
    fitReport,
    fitScoreCacheHit,
    fitScoreLoading,
    analyzeFit,
    contextChangedSinceAnalysis,
  } = useResumeStore();
  if (!fitReport) return null;

  const scoreColor =
    fitReport.overall_score >= 70
      ? "text-green-600"
      : fitReport.overall_score >= 50
      ? "text-yellow-600"
      : "text-destructive";

  return (
    <div className="border rounded-lg p-3 flex flex-col gap-3">
      {/* Stale score banner — shown when context has been updated since last analysis */}
      {contextChangedSinceAnalysis && fitReport && (
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

      {/* Score header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium">Fit Score</span>
          <span className="text-xs text-muted-foreground">
            via {fitReport.scorer_backend}
          </span>
          {fitScoreCacheHit === true && (
            <Badge variant="outline" className="text-xs">
              cached
            </Badge>
          )}
          {fitScoreCacheHit === false && (
            <Badge variant="outline" className="text-xs">
              live
            </Badge>
          )}
        </div>
        <span className={`text-2xl font-bold ${scoreColor}`}>
          {fitReport.overall_score}
          <span className="text-sm font-normal text-muted-foreground">
            /100
          </span>
        </span>
      </div>

      {/* Recommendation */}
      {fitReport.recommendation && (
        <p className="text-xs text-muted-foreground italic">
          {fitReport.recommendation}
        </p>
      )}

      {/* Strong matches */}
      {fitReport.strong_matches.length > 0 && (
        <div>
          <p className="text-xs font-semibold mb-1 text-muted-foreground uppercase tracking-wider">
            Strong Matches
          </p>
          <div className="flex flex-wrap gap-1">
            {fitReport.strong_matches.slice(0, 8).map((m: FitMatch) => (
              <Badge
                key={m.dimension}
                variant="default"
                className="text-xs"
                title={m.jd_requirement}
              >
                {m.dimension}
              </Badge>
            ))}
          </div>
        </div>
      )}

      {/* Partial matches */}
      {fitReport.partial_matches.length > 0 && (
        <div>
          <p className="text-xs font-semibold mb-1 text-muted-foreground uppercase tracking-wider">
            Partial Matches
          </p>
          <div className="flex flex-wrap gap-1">
            {fitReport.partial_matches.slice(0, 6).map((m: FitMatch) => (
              <Badge
                key={m.dimension}
                variant="secondary"
                className="text-xs"
                title={m.jd_requirement}
              >
                {m.dimension}
              </Badge>
            ))}
          </div>
        </div>
      )}

      {/* Gaps */}
      {fitReport.gaps.length > 0 && (
        <div>
          <p className="text-xs font-semibold mb-1 text-muted-foreground uppercase tracking-wider">
            Gaps
          </p>
          <div className="flex flex-wrap gap-1">
            {fitReport.gaps.slice(0, 5).map((g: Gap) => (
              <Badge
                key={g.keyword}
                variant="outline"
                className="text-xs text-destructive border-destructive/40"
              >
                {g.keyword}
              </Badge>
            ))}
          </div>
        </div>
      )}

      {/* Cache footer — shown when score is from cache */}
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
