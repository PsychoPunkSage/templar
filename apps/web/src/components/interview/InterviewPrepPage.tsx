"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";
import { Brain, ArrowLeft, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useInterviewPrepStore } from "@/store/interviewPrepStore";
import { PrepStatusBanner } from "./PrepStatusBanner";
import { QuestionList } from "./QuestionList";
import { StarScaffoldViewer } from "./StarScaffold";
import { BulletDeepDive } from "./BulletDeepDive";
import { CompanyContextForm } from "./CompanyContextForm";
import type { CompanyContext } from "@templar/types";

interface InterviewPrepPageProps {
  projectId: string;
}

export function InterviewPrepPage({ projectId }: InterviewPrepPageProps) {
  const router = useRouter();
  const {
    status,
    meta,
    bullets,
    selectedBulletHash,
    questionFilter,
    isTriggering,
    error,
    loadPrep,
    triggerPrep,
    updateCompany,
    selectBullet,
    setFilter,
    clearError,
  } = useInterviewPrepStore();

  useEffect(() => {
    loadPrep(projectId);
    // Stop polling when the component unmounts or projectId changes.
    // The poll interval is module-scoped in the store — without this cleanup
    // it would keep firing after navigation.
    return () => {
      useInterviewPrepStore.getState().stopPolling();
    };
  }, [projectId, loadPrep]);

  const selectedBullet = bullets.find((b) => b.bullet_hash === selectedBulletHash) ?? null;
  const gapQuestions = meta?.gap_questions ?? [];

  // "idle" or "pending" with no bullets = nothing generated yet
  const isPristine =
    (status === "idle" || status === "pending") && bullets.length === 0;

  return (
    <div className="flex flex-col h-screen">
      {/* Top bar */}
      <div className="flex items-center justify-between px-6 py-3 border-b border-border shrink-0">
        <div className="flex items-center gap-3">
          <button
            onClick={() => router.push(`/editor/${projectId}`)}
            className="text-muted-foreground hover:text-foreground transition-colors"
            aria-label="Back to editor"
          >
            <ArrowLeft className="h-4 w-4" />
          </button>
          <Brain className="h-5 w-5 text-indigo-500" />
          <h1 className="text-base font-semibold text-foreground">Interview Prep</h1>
          {meta?.company_context?.company_name && (
            <span className="rounded-full bg-muted px-2.5 py-0.5 text-xs text-muted-foreground">
              {meta.company_context.company_name}
              {meta.company_context.role_title ? ` — ${meta.company_context.role_title}` : ""}
            </span>
          )}
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => triggerPrep(projectId)}
          disabled={status === "generating" || isTriggering}
          className="gap-1.5"
        >
          <RefreshCw className="h-3.5 w-3.5" />
          {status === "generating" ? "Updating..." : "Refresh prep"}
        </Button>
      </div>

      {/* Status banner */}
      <PrepStatusBanner
        status={status}
        isStale={meta?.is_stale ?? false}
        expiresAt={meta?.expires_at}
        onTrigger={() => triggerPrep(projectId)}
        isTriggering={isTriggering}
      />

      {/* Error banner */}
      {error && (
        <div className="px-6 py-2.5 bg-destructive/10 text-destructive text-sm flex items-center justify-between border-b shrink-0">
          <span>{error}</span>
          <button onClick={clearError} className="text-xs underline hover:no-underline ml-4">
            Dismiss
          </button>
        </div>
      )}

      {/* Pristine state */}
      {isPristine && (
        <div className="flex-1 flex items-center justify-center">
          <div className="text-center space-y-4 max-w-sm">
            <Brain className="h-12 w-12 text-muted-foreground/40 mx-auto" />
            <div>
              <h2 className="text-base font-semibold text-foreground mb-1">
                No interview prep yet
              </h2>
              <p className="text-sm text-muted-foreground">
                Generate interview prep to get STAR scaffolds, behavioral questions,
                and gap questions tailored to your resume and job description.
              </p>
            </div>
            <Button
              onClick={() => triggerPrep(projectId)}
              disabled={isTriggering}
              className="gap-2"
            >
              <Brain className="h-4 w-4" />
              {isTriggering ? "Starting..." : "Generate interview prep"}
            </Button>
          </div>
        </div>
      )}

      {/* Main content — only when we have data */}
      {!isPristine && bullets.length > 0 && (
        <div className="flex-1 flex overflow-hidden">
          {/* Left pane: question list (40%) */}
          <div className="w-[40%] flex flex-col border-r border-border min-h-0">
            <div className="px-4 py-3 border-b border-border shrink-0">
              <h2 className="text-sm font-semibold text-foreground">Questions</h2>
              <p className="text-xs text-muted-foreground mt-0.5">
                Select a bullet to view its STAR scaffold and deep-dive questions.
              </p>
            </div>
            <div className="flex-1 overflow-y-auto p-4">
              <QuestionList
                bullets={bullets}
                gapQuestions={gapQuestions}
                selectedHash={selectedBulletHash}
                filter={questionFilter}
                onSelectBullet={selectBullet}
                onFilterChange={setFilter}
              />
            </div>
          </div>

          {/* Right pane: detail (60%) */}
          <div className="w-[60%] flex flex-col min-h-0">
            <ScrollArea className="flex-1 h-0">
              <div className="p-6 space-y-6">
                {selectedBullet ? (
                  <>
                    <section>
                      <h3 className="text-sm font-semibold text-foreground mb-3">STAR scaffold</h3>
                      <StarScaffoldViewer
                        scaffold={selectedBullet.star_scaffold}
                        bulletText={selectedBullet.bullet_text}
                      />
                    </section>

                    <section>
                      <h3 className="text-sm font-semibold text-foreground mb-3">
                        Interview questions for this bullet
                      </h3>
                      <BulletDeepDive
                        questions={selectedBullet.questions}
                        activeFilter={questionFilter}
                      />
                    </section>
                  </>
                ) : questionFilter === "gap" && gapQuestions.length > 0 ? (
                  <section>
                    <h3 className="text-sm font-semibold text-foreground mb-3">Gap questions</h3>
                    <div className="space-y-3">
                      {gapQuestions.map((gq, i) => (
                        <div
                          key={i}
                          className="rounded-md border border-border bg-card px-3 py-2.5 space-y-1"
                        >
                          <p className="text-sm text-foreground">{gq.text}</p>
                          <span className="text-xs text-amber-600 dark:text-amber-400">
                            Gap area: {gq.gap_area}
                          </span>
                        </div>
                      ))}
                    </div>
                  </section>
                ) : (
                  <div className="flex items-center justify-center h-40 text-muted-foreground">
                    <p className="text-sm italic">Select a bullet on the left to see its STAR scaffold and questions.</p>
                  </div>
                )}

                {/* Company context form — always visible at bottom of right pane */}
                <section>
                  <CompanyContextForm
                    current={meta?.company_context}
                    onSave={(ctx: CompanyContext) => updateCompany(projectId, ctx)}
                  />
                </section>
              </div>
            </ScrollArea>
          </div>
        </div>
      )}

      {/* Generating state with no existing data */}
      {status === "generating" && bullets.length === 0 && (
        <div className="flex-1 flex items-center justify-center text-muted-foreground">
          <p className="text-sm animate-pulse">Generating interview prep...</p>
        </div>
      )}
    </div>
  );
}
