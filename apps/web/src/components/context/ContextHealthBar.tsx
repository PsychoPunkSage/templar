'use client'

import { useState } from 'react'
import type { CompletenessReport, CompletenessSection } from '@/lib/api'

// ─── Helpers ────────────────────────────────────────────────────────────────

function statusColor(status: CompletenessSection['status']): string {
  switch (status) {
    case 'strong':
      return 'bg-green-500'
    case 'moderate':
      return 'bg-amber-500'
    case 'weak':
      return 'bg-orange-500'
    case 'missing':
      return 'bg-red-400'
  }
}

function statusText(status: CompletenessSection['status']): string {
  switch (status) {
    case 'strong':
      return 'text-green-600 dark:text-green-400'
    case 'moderate':
      return 'text-amber-600 dark:text-amber-400'
    case 'weak':
      return 'text-orange-600 dark:text-orange-400'
    case 'missing':
      return 'text-red-500 dark:text-red-400'
  }
}

function scoreBarColor(score: number): string {
  if (score >= 0.8) return 'bg-green-500'
  if (score >= 0.5) return 'bg-amber-500'
  if (score >= 0.2) return 'bg-orange-500'
  return 'bg-red-400'
}

// ─── Sub-component: section row in the expanded panel ───────────────────────

function SectionRow({ section }: { section: CompletenessSection }) {
  const pct = Math.round(section.score * 100)
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between text-xs">
        <div className="flex items-center gap-2">
          <span className="font-medium capitalize">{section.section.replace('_', ' ')}</span>
          <span className="text-muted-foreground">
            {section.entry_count} {section.entry_count === 1 ? 'entry' : 'entries'}
          </span>
        </div>
        <span className={`capitalize font-medium ${statusText(section.status)}`}>
          {section.status}
        </span>
      </div>

      {/* Progress bar */}
      <div className="w-full h-1.5 bg-muted rounded-full overflow-hidden">
        <div
          className={`h-full rounded-full transition-all duration-300 ${scoreBarColor(section.score)}`}
          style={{ width: `${pct}%` }}
        />
      </div>

      {/* Recommendations */}
      {section.recommendations.length > 0 && (
        <ul className="space-y-0.5">
          {section.recommendations.map((rec, i) => (
            <li key={i} className="text-xs text-muted-foreground leading-snug">
              {rec}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

// ─── Main component ─────────────────────────────────────────────────────────

interface ContextHealthBarProps {
  completeness: CompletenessReport
}

export default function ContextHealthBar({ completeness }: ContextHealthBarProps) {
  const [expanded, setExpanded] = useState(false)

  const pct = Math.round(completeness.overall_score * 100)
  const filledBars = Math.round((pct / 100) * 10)
  const emptyBars = 10 - filledBars

  const barColor =
    pct >= 80
      ? 'bg-green-500'
      : pct >= 50
      ? 'bg-amber-500'
      : pct >= 20
      ? 'bg-orange-500'
      : 'bg-red-400'

  return (
    <div className="rounded-lg border bg-card p-4 space-y-3">
      {/* Single-row summary */}
      <button
        onClick={() => setExpanded((v) => !v)}
        className="w-full flex items-center gap-3 text-sm hover:opacity-80 transition-opacity"
        aria-expanded={expanded}
      >
        <span className="font-medium shrink-0">Context Health</span>

        {/* Block progress bar (ASCII-style) */}
        <div className="flex items-center gap-px shrink-0" aria-hidden>
          {Array.from({ length: filledBars }).map((_, i) => (
            <div key={`f${i}`} className={`w-3 h-3 rounded-sm ${barColor}`} />
          ))}
          {Array.from({ length: emptyBars }).map((_, i) => (
            <div key={`e${i}`} className="w-3 h-3 rounded-sm bg-muted" />
          ))}
        </div>

        <span className={`font-bold tabular-nums shrink-0 ${
          pct >= 80
            ? 'text-green-600 dark:text-green-400'
            : pct >= 50
            ? 'text-amber-600 dark:text-amber-400'
            : 'text-red-500 dark:text-red-400'
        }`}>
          {pct}%
        </span>

        <span className="text-muted-foreground shrink-0">
          {completeness.total_entries} {completeness.total_entries === 1 ? 'entry' : 'entries'}
        </span>

        {completeness.missing_sections.length > 0 && (
          <span className="text-muted-foreground truncate min-w-0">
            Missing: {completeness.missing_sections.join(', ')}
          </span>
        )}

        {/* Chevron */}
        <span className="ml-auto shrink-0 text-muted-foreground text-xs">
          {expanded ? '▲' : '▼'}
        </span>
      </button>

      {/* Expanded per-section detail */}
      {expanded && (
        <div className="border-t pt-3 space-y-4">
          {completeness.sections.map((section) => (
            <SectionRow key={section.section} section={section} />
          ))}
        </div>
      )}
    </div>
  )
}
