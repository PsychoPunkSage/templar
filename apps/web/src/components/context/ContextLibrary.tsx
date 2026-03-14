'use client'

import { useCallback, useEffect, useState } from 'react'
import { api } from '@/lib/api'
import type { ContextEntryRow, CompletenessReport } from '@/lib/api'
import ContextHealthBar from './ContextHealthBar'
import ContextEntryCard from './ContextEntryCard'

// ─── Constants ───────────────────────────────────────────────────────────────

const MVP_USER_ID = '00000000-0000-0000-0000-000000000001'

/** Canonical display order for entry_type groups. */
const SECTION_ORDER = [
  'experience',
  'education',
  'project',
  'skill',
  'certification',
  'publication',
  'open_source',
  'award',
  'extracurricular',
]

// ─── Helpers ─────────────────────────────────────────────────────────────────

function groupEntries(
  entries: ContextEntryRow[],
  filter: string
): Map<string, ContextEntryRow[]> {
  const filtered =
    filter === 'all' ? entries : entries.filter((e) => e.entry_type === filter)

  const grouped = new Map<string, ContextEntryRow[]>()

  // Insert in canonical order first
  for (const section of SECTION_ORDER) {
    const sectionEntries = filtered.filter((e) => e.entry_type === section)
    if (sectionEntries.length > 0) {
      grouped.set(section, sectionEntries)
    }
  }

  // Any types not in SECTION_ORDER go at the end
  const knownTypes = new Set(SECTION_ORDER)
  for (const entry of filtered) {
    if (!knownTypes.has(entry.entry_type)) {
      const existing = grouped.get(entry.entry_type) ?? []
      existing.push(entry)
      grouped.set(entry.entry_type, existing)
    }
  }

  return grouped
}

function sectionLabel(type: string): string {
  return type.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase())
}

function uniqueTypes(entries: ContextEntryRow[]): string[] {
  const seen = new Set<string>()
  const ordered: string[] = []
  for (const s of SECTION_ORDER) {
    if (entries.some((e) => e.entry_type === s)) {
      seen.add(s)
      ordered.push(s)
    }
  }
  for (const e of entries) {
    if (!seen.has(e.entry_type)) {
      seen.add(e.entry_type)
      ordered.push(e.entry_type)
    }
  }
  return ordered
}

// ─── Skeleton loading state ───────────────────────────────────────────────────

function SkeletonCard() {
  return (
    <div className="border rounded-lg p-4 space-y-3 animate-pulse">
      <div className="flex gap-3">
        <div className="h-5 w-20 rounded bg-muted" />
        <div className="flex-1 space-y-2">
          <div className="h-4 w-3/4 rounded bg-muted" />
          <div className="h-3 w-1/2 rounded bg-muted" />
        </div>
      </div>
      <div className="h-1.5 w-full rounded bg-muted" />
      <div className="flex gap-2">
        <div className="h-5 w-16 rounded bg-muted" />
        <div className="h-5 w-12 rounded bg-muted" />
        <div className="h-5 w-20 rounded bg-muted" />
      </div>
    </div>
  )
}

// ─── Main component ───────────────────────────────────────────────────────────

interface ContextLibraryProps {
  /** Incrementing this triggers a re-fetch. Controlled by the parent page. */
  refreshKey: number
}

export default function ContextLibrary({ refreshKey }: ContextLibraryProps) {
  const [entries, setEntries] = useState<ContextEntryRow[]>([])
  const [completeness, setCompleteness] = useState<CompletenessReport | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const [filter, setFilter] = useState<string>('all')

  const fetchEntries = useCallback(async () => {
    setIsLoading(true)
    setError(null)
    try {
      const resp = await api.getContextEntries(MVP_USER_ID)
      setEntries(resp.entries)
      setCompleteness(resp.completeness)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load context entries')
    } finally {
      setIsLoading(false)
    }
  }, [])

  // Fetch on mount and whenever refreshKey changes.
  useEffect(() => {
    fetchEntries()
  }, [fetchEntries, refreshKey])

  // Reset filter if there are no entries of the selected type after refresh.
  useEffect(() => {
    if (filter !== 'all' && !entries.some((e) => e.entry_type === filter)) {
      setFilter('all')
    }
  }, [entries, filter])

  const availableTypes = uniqueTypes(entries)
  const grouped = groupEntries(entries, filter)

  // ── Render ────────────────────────────────────────────────────────────────

  return (
    <div className="space-y-4">
      {/* Health bar */}
      {completeness && !isLoading && (
        <ContextHealthBar completeness={completeness} />
      )}

      {/* Filter + count row */}
      {!isLoading && entries.length > 0 && (
        <div className="flex items-center gap-3">
          <span className="text-sm text-muted-foreground">Filter</span>
          <div className="flex flex-wrap gap-2">
            <button
              onClick={() => setFilter('all')}
              className={`text-xs px-2.5 py-1 rounded-md border transition-colors ${
                filter === 'all'
                  ? 'border-primary bg-primary/10 text-primary font-medium'
                  : 'border-border text-muted-foreground hover:border-primary/50'
              }`}
            >
              All ({entries.length})
            </button>
            {availableTypes.map((type) => {
              const count = entries.filter((e) => e.entry_type === type).length
              return (
                <button
                  key={type}
                  onClick={() => setFilter(type)}
                  className={`text-xs px-2.5 py-1 rounded-md border capitalize transition-colors ${
                    filter === type
                      ? 'border-primary bg-primary/10 text-primary font-medium'
                      : 'border-border text-muted-foreground hover:border-primary/50'
                  }`}
                >
                  {sectionLabel(type)} ({count})
                </button>
              )
            })}
          </div>
        </div>
      )}

      {/* Loading skeletons */}
      {isLoading && (
        <div className="space-y-3">
          <SkeletonCard />
          <SkeletonCard />
          <SkeletonCard />
        </div>
      )}

      {/* Error state */}
      {!isLoading && error && (
        <div className="rounded-lg border border-destructive/40 bg-destructive/5 p-4 text-sm text-destructive">
          <p className="font-medium">Failed to load context</p>
          <p className="text-xs mt-1 text-muted-foreground">{error}</p>
          <button
            onClick={fetchEntries}
            className="mt-2 text-xs underline underline-offset-2 hover:text-destructive"
          >
            Retry
          </button>
        </div>
      )}

      {/* Empty state */}
      {!isLoading && !error && entries.length === 0 && (
        <div className="rounded-lg border border-dashed p-10 text-center space-y-2">
          <p className="text-sm font-medium">No context entries yet</p>
          <p className="text-xs text-muted-foreground max-w-xs mx-auto">
            Paste your resume, work experience, or project descriptions in the panel
            on the left to get started.
          </p>
        </div>
      )}

      {/* Entry groups */}
      {!isLoading && !error && entries.length > 0 && (
        <div className="space-y-6">
          {Array.from(grouped.entries()).map(([type, sectionEntries]) => (
            <div key={type} className="space-y-2">
              {/* Section header */}
              <h2 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                {sectionLabel(type)} &middot; {sectionEntries.length}{' '}
                {sectionEntries.length === 1 ? 'entry' : 'entries'}
              </h2>

              {/* Cards */}
              <div className="space-y-2">
                {sectionEntries.map((entry) => (
                  <ContextEntryCard
                    key={entry.id}
                    entry={entry}
                    isExpanded={expandedId === entry.id}
                    onToggle={() =>
                      setExpandedId((prev) => (prev === entry.id ? null : entry.id))
                    }
                  />
                ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
