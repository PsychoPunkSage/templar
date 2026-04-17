"use client";

import { useResumeStore } from "@/store/resumeStore";
import { BulletCard } from "./BulletCard";
import type { EntryDisplayHeader, EntryGroup } from "@templar/types";

/** Returns a human-readable one-line summary for the entry header row. */
function formatDisplayHeader(h: EntryDisplayHeader): string {
  switch (h.type) {
    case "experience":
      return `${h.company} — ${h.role}  ·  ${h.date_range}`;
    case "project":
      return h.name;
    case "open_source":
      return h.project_name;
    case "education":
      return `${h.institution}  ·  ${h.degree}  ·  ${h.date_range}`;
    case "skills":
      return h.category;
    case "other":
      return h.label;
  }
}

/**
 * Groups entry groups by section, then renders them as:
 *   Section header → entry header row → BulletCard per bullet
 *
 * Consumes entryGroups from the resume store (populated by generate() + loadResume()).
 */
export function BulletList() {
  const { entryGroups, auditManifest } = useResumeStore();

  if (entryGroups.length === 0) {
    return (
      <p className="text-sm text-muted-foreground italic">
        No bullets yet. Paste a job description and click Generate Resume.
      </p>
    );
  }

  // Group EntryGroups by section, preserving their original order
  const sectionOrder: string[] = [];
  const bySection = entryGroups.reduce<Record<string, EntryGroup[]>>((acc, g) => {
    if (!acc[g.section]) {
      sectionOrder.push(g.section);
      acc[g.section] = [];
    }
    acc[g.section].push(g);
    return acc;
  }, {});

  // Build a lookup map from bullet text to audit entry
  const auditMap = new Map(
    (auditManifest?.entries ?? []).map((e) => [e.bullet_text, e])
  );

  return (
    <div className="flex flex-col gap-4">
      {sectionOrder.map((section) => (
        <div key={section}>
          {/* Section header */}
          <div className="flex items-center gap-2 mb-2">
            <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground shrink-0">
              {section}
            </h3>
            <div className="flex-1 border-t border-border" />
          </div>

          {/* Entry groups within this section */}
          <div className="flex flex-col gap-3">
            {bySection[section].map((group) => (
              <div key={group.source_entry_id}>
                {/* Entry header row — company/role/dates or project name */}
                <p className="text-xs font-medium text-foreground/70 mb-1.5 truncate">
                  {formatDisplayHeader(group.display_header)}
                </p>

                {/* Bullets for this entry */}
                <div className="flex flex-col gap-2 pl-2 border-l border-border/50">
                  {group.bullets.map((b, i) => (
                    <BulletCard
                      key={`${b.source_entry_id}-${i}`}
                      bullet={b}
                      auditEntry={auditMap.get(b.text)}
                    />
                  ))}
                </div>
              </div>
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
