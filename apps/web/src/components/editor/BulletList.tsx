"use client";

import { useResumeStore } from "@/store/resumeStore";
import { BulletCard } from "./BulletCard";
import type { SimulatedBullet } from "@templar/types";

/**
 * Groups bullets by section and renders them as BulletCards with
 * their corresponding audit entries (when available).
 */
export function BulletList() {
  const { bullets, auditManifest } = useResumeStore();

  if (bullets.length === 0) {
    return (
      <p className="text-sm text-muted-foreground italic">
        No bullets yet. Paste a job description and click Generate Resume.
      </p>
    );
  }

  // Group bullets by section, preserving order within each section
  const sections = bullets.reduce<Record<string, SimulatedBullet[]>>(
    (acc, b) => {
      if (!acc[b.section]) acc[b.section] = [];
      acc[b.section].push(b);
      return acc;
    },
    {}
  );

  // Build a lookup map from bullet text to audit entry
  const auditMap = new Map(
    (auditManifest?.entries ?? []).map((e) => [e.bullet_text, e])
  );

  return (
    <div className="flex flex-col gap-4">
      {Object.entries(sections).map(([section, sectionBullets]) => (
        <div key={section}>
          <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-2">
            {section}
          </h3>
          <div className="flex flex-col gap-2">
            {sectionBullets.map((b, i) => (
              <BulletCard
                key={`${b.source_entry_id}-${i}`}
                bullet={b}
                auditEntry={auditMap.get(b.text)}
              />
            ))}
          </div>
        </div>
      ))}

      {auditManifest && (
        <div className="text-xs text-muted-foreground border-t pt-3 mt-1 flex gap-4">
          <span>
            Pass rate:{" "}
            {(auditManifest.overall_pass_rate * 100).toFixed(0)}%
          </span>
          {auditManifest.bullets_flagged > 0 && (
            <span className="text-yellow-600">
              {auditManifest.bullets_flagged} flagged
            </span>
          )}
          {auditManifest.bullets_rejected > 0 && (
            <span className="text-destructive">
              {auditManifest.bullets_rejected} rejected
            </span>
          )}
        </div>
      )}
    </div>
  );
}
