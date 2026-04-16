"use client";

import { useState, useRef, useEffect } from "react";
import { Loader2, Pencil } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { useResumeStore } from "@/store/resumeStore";
import type { SimulatedBullet, AuditEntry } from "@templar/types";

interface BulletCardProps {
  bullet: SimulatedBullet;
  auditEntry?: AuditEntry;
}

type VerdictKey = "pass" | "flag_for_review" | "fail";

const VERDICT_CONFIG = {
  pass: {
    label: "Grounded",
    variant: "outline" as const,
    borderClass: "border-l-green-500/60",
    badgeClass:  "border-green-500/60 text-green-700 bg-green-50 dark:bg-green-900/20 dark:text-green-400",
    chipClass:   "bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400",
  },
  flag_for_review: {
    label: "Review",
    variant: "outline" as const,
    borderClass: "border-l-amber-500",
    badgeClass:  "border-amber-500/60 text-amber-700 bg-amber-50 dark:bg-amber-900/20 dark:text-amber-400",
    chipClass:   "bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400",
  },
  fail: {
    label: "Ungrounded",
    variant: "outline" as const,
    borderClass: "border-l-destructive",
    badgeClass:  "border-red-500/60 text-red-700 bg-red-50 dark:bg-red-900/20 dark:text-red-400",
    chipClass:   "bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400",
  },
};

export function BulletCard({ bullet, auditEntry }: BulletCardProps) {
  const verdict = auditEntry?.verdict ?? (bullet.flagged_for_review ? "flag_for_review" : "pass");
  const config = VERDICT_CONFIG[verdict as VerdictKey] ?? VERDICT_CONFIG.pass;

  const { refiningBullets, refinementQueue, refineBullet, queueRefinement, removeFromQueue } =
    useResumeStore();

  const isRefining  = refiningBullets.includes(bullet.text);
  const queuedItem  = refinementQueue.find((r) => r.bulletText === bullet.text);

  const [showInput, setShowInput]       = useState(false);
  const [instruction, setInstruction]   = useState("");
  const [rejectionMsg, setRejectionMsg] = useState<string | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => { if (showInput) inputRef.current?.focus(); }, [showInput]);

  useEffect(() => {
    if (!rejectionMsg) return;
    const t = setTimeout(() => setRejectionMsg(null), 4000);
    return () => clearTimeout(t);
  }, [rejectionMsg]);

  const handleRefine = async () => {
    if (!instruction.trim()) return;
    setShowInput(false);
    try {
      await refineBullet(bullet.text, bullet.source_entry_id, bullet.section, instruction);
      setInstruction("");
    } catch (e) {
      setRejectionMsg(e instanceof Error ? e.message : "Refinement rejected — grounding too low");
    }
  };

  const handleQueue = () => {
    if (!instruction.trim()) return;
    queueRefinement(bullet.text, bullet.source_entry_id, bullet.section, instruction);
    setInstruction("");
    setShowInput(false);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); handleRefine(); }
    if (e.key === "Escape") { setShowInput(false); setInstruction(""); }
  };

  return (
    <Card className={`border-l-2 ${config.borderClass} relative`}>
      {/* In-flight overlay */}
      {isRefining && (
        <div className="absolute inset-0 bg-background/70 flex items-center justify-center rounded z-10">
          <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
        </div>
      )}

      <CardContent className="p-3 flex flex-col gap-2">
        {/* Bullet text */}
        <p className="text-sm leading-snug">{bullet.text}</p>

        {/* Metadata row */}
        <div className="flex items-center gap-2 flex-wrap">
          <Badge variant={config.variant} className={`text-xs ${config.badgeClass}`}>
            {config.label}
          </Badge>

          {auditEntry && (
            <span className={`inline-flex items-center rounded-full px-1.5 py-0.5 text-xs font-medium tabular-nums ${config.chipClass}`}>
              {(auditEntry.composite_score * 100).toFixed(0)}%
            </span>
          )}

          <span className="text-xs text-muted-foreground tabular-nums">
            {bullet.verified_line_count}L
          </span>

          {bullet.was_adjusted && (
            <span className="inline-flex items-center rounded-full px-1.5 py-0.5 text-xs bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300">
              Adjusted
            </span>
          )}
        </div>

        {auditEntry?.rejection_reason && (
          <p className="text-xs text-destructive">{auditEntry.rejection_reason}</p>
        )}

        {rejectionMsg && (
          <p className="text-xs text-destructive font-medium">{rejectionMsg}</p>
        )}

        {/* Queued badge */}
        {queuedItem && !isRefining && (
          <div className="flex items-center justify-between px-2 py-1.5 rounded-md bg-muted/60 text-xs text-muted-foreground">
            <span className="truncate">
              Queued: <span className="italic">{queuedItem.instruction}</span>
            </span>
            <button
              onClick={() => removeFromQueue(bullet.text)}
              className="ml-2 shrink-0 hover:text-foreground transition-colors"
              aria-label="Remove from queue"
            >
              ×
            </button>
          </div>
        )}

        {/* ── Refine section — always visible, clearly separated ── */}
        {!isRefining && !queuedItem && (
          <div className="border-t border-border/40 pt-2">
            {!showInput ? (
              /* Collapsed: always-visible refine trigger */
              <button
                onClick={() => setShowInput(true)}
                className="flex items-center gap-1.5 text-xs text-muted-foreground hover:text-foreground transition-colors w-full text-left"
              >
                <Pencil className="h-3 w-3 shrink-0" />
                <span>Refine this bullet…</span>
              </button>
            ) : (
              /* Expanded: instruction input + action buttons */
              <div className="flex flex-col gap-2">
                <textarea
                  ref={inputRef}
                  value={instruction}
                  onChange={(e) => setInstruction(e.target.value)}
                  onKeyDown={handleKeyDown}
                  placeholder={`"make this more concise" · "add the Rust part" · "stronger verb"`}
                  rows={2}
                  className="w-full text-xs px-2 py-1.5 rounded-md border border-border bg-muted/30 resize-none focus:outline-none focus:ring-1 focus:ring-ring placeholder:text-muted-foreground/60"
                />
                <div className="flex gap-1.5">
                  <button
                    onClick={handleRefine}
                    disabled={!instruction.trim()}
                    className="text-xs px-2.5 py-1 rounded-md bg-primary text-primary-foreground disabled:opacity-40 hover:opacity-90 transition-opacity"
                  >
                    Refine ↵
                  </button>
                  <button
                    onClick={handleQueue}
                    disabled={!instruction.trim()}
                    className="text-xs px-2.5 py-1 rounded-md border border-border text-muted-foreground disabled:opacity-40 hover:text-foreground transition-colors"
                  >
                    + Queue
                  </button>
                  <button
                    onClick={() => { setShowInput(false); setInstruction(""); }}
                    className="text-xs px-2 py-1 text-muted-foreground hover:text-foreground transition-colors"
                  >
                    Cancel
                  </button>
                </div>
              </div>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
