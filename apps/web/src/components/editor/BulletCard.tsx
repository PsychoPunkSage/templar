"use client";

import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import type { SimulatedBullet, AuditEntry } from "@templar/types";

interface BulletCardProps {
  bullet: SimulatedBullet;
  auditEntry?: AuditEntry;
}

type VerdictKey = "pass" | "flag_for_review" | "fail";

interface VerdictConfig {
  label: string;
  variant: "default" | "secondary" | "destructive";
  borderClass: string;
}

const VERDICT_CONFIG: Record<VerdictKey, VerdictConfig> = {
  pass: { label: "Grounded", variant: "default", borderClass: "border-l-green-500/60" },
  flag_for_review: { label: "Review", variant: "secondary", borderClass: "border-l-amber-500" },
  fail: { label: "Ungrounded", variant: "destructive", borderClass: "border-l-destructive" },
};

function getVerdictConfig(verdict: string): VerdictConfig {
  return VERDICT_CONFIG[verdict as VerdictKey] ?? VERDICT_CONFIG.pass;
}

/**
 * Displays a single resume bullet with grounding status, line count,
 * and layout adjustment indicators.
 */
export function BulletCard({ bullet, auditEntry }: BulletCardProps) {
  const verdict = auditEntry?.verdict ?? (bullet.flagged_for_review ? "flag_for_review" : "pass");
  const config = getVerdictConfig(verdict);

  return (
    <Card className={`border-l-2 ${config.borderClass}`}>
      <CardContent className="p-3 flex flex-col gap-1">
        <p className="text-sm leading-snug">{bullet.text}</p>
        <div className="flex items-center gap-2 flex-wrap mt-1">
          <Badge variant={config.variant} className="text-xs">
            {config.label}
          </Badge>

          {auditEntry && (
            <span className="text-xs text-muted-foreground">
              {(auditEntry.composite_score * 100).toFixed(0)}% grounded
            </span>
          )}

          <span className="text-xs text-muted-foreground tabular-nums">
            {bullet.verified_line_count}L
          </span>

          {bullet.flagged_for_review && (
            <span className="text-xs text-amber-600 font-medium">
              Review needed
            </span>
          )}

          {bullet.was_adjusted && (
            <span className="inline-flex items-center rounded-full px-1.5 py-0.5 text-xs bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300">
              Adjusted
            </span>
          )}
        </div>

        {auditEntry?.rejection_reason && (
          <p className="text-xs text-destructive mt-1">
            {auditEntry.rejection_reason}
          </p>
        )}
      </CardContent>
    </Card>
  );
}
