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
}

const VERDICT_CONFIG: Record<VerdictKey, VerdictConfig> = {
  pass: { label: "Grounded", variant: "default" },
  flag_for_review: { label: "Review", variant: "secondary" },
  fail: { label: "Ungrounded", variant: "destructive" },
};

function getVerdictConfig(verdict: string): VerdictConfig {
  return VERDICT_CONFIG[verdict as VerdictKey] ?? VERDICT_CONFIG.pass;
}

/**
 * Displays a single resume bullet with grounding status, line count,
 * and layout adjustment indicators.
 */
export function BulletCard({ bullet, auditEntry }: BulletCardProps) {
  const verdict = auditEntry?.verdict ?? "pass";
  const config = getVerdictConfig(verdict);

  return (
    <Card
      className={
        bullet.flagged_for_review ? "border-yellow-500" : undefined
      }
    >
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

          <span className="text-xs text-muted-foreground">
            {bullet.verified_line_count}L
          </span>

          {bullet.flagged_for_review && (
            <span className="text-xs text-yellow-600 font-medium">
              Layout review needed
            </span>
          )}

          {bullet.was_adjusted && (
            <span className="text-xs text-blue-500">Adjusted</span>
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
