'use client'

import { useState } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { api } from '@/lib/api'
import type { ContextEntryRow } from '@/lib/api'

// ─── Types ───────────────────────────────────────────────────────────────────

type BulletItem = {
  text: string
  impact_markers?: string[]
  confidence_marker?: string | null
}

function normalizeBullets(raw: unknown[]): BulletItem[] {
  return raw.map(b => {
    if (typeof b === 'string') return { text: b }
    if (typeof b === 'object' && b !== null && 'text' in b) return b as BulletItem
    return { text: String(b) }
  })
}

// ─── Constants ───────────────────────────────────────────────────────────────

const MVP_USER_ID = '00000000-0000-0000-0000-000000000001'

/** Max tags shown in collapsed view before truncating. */
const MAX_COLLAPSED_TAGS = 4

/** Array-typed fields rendered as tag pills in the expanded view. */
const TAG_ARRAY_KEYS = new Set(['tech_stack', 'items', 'honors', 'relevant_courses', 'authors'])

// ─── Type badge colors ───────────────────────────────────────────────────────

const TYPE_BADGE_CLASSES: Record<string, string> = {
  experience:
    'bg-blue-100 text-blue-800 dark:bg-blue-900/40 dark:text-blue-300',
  education:
    'bg-purple-100 text-purple-800 dark:bg-purple-900/40 dark:text-purple-300',
  project:
    'bg-green-100 text-green-800 dark:bg-green-900/40 dark:text-green-300',
  skill:
    'bg-cyan-100 text-cyan-800 dark:bg-cyan-900/40 dark:text-cyan-300',
  certification:
    'bg-orange-100 text-orange-800 dark:bg-orange-900/40 dark:text-orange-300',
  publication:
    'bg-violet-100 text-violet-800 dark:bg-violet-900/40 dark:text-violet-300',
  open_source:
    'bg-teal-100 text-teal-800 dark:bg-teal-900/40 dark:text-teal-300',
  award:
    'bg-yellow-100 text-yellow-800 dark:bg-yellow-900/40 dark:text-yellow-300',
  extracurricular:
    'bg-pink-100 text-pink-800 dark:bg-pink-900/40 dark:text-pink-300',
}

function typeBadgeClass(entryType: string): string {
  return (
    TYPE_BADGE_CLASSES[entryType] ??
    'bg-gray-100 text-gray-800 dark:bg-gray-800 dark:text-gray-300'
  )
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

function qualityBarColor(score: number): string {
  if (score >= 0.8) return 'bg-green-500'
  if (score >= 0.65) return 'bg-amber-500'
  return 'bg-red-500'
}

function getTitle(entry: ContextEntryRow): string {
  const d = entry.data ?? {}
  return (
    (d.company as string) ||
    (d.institution as string) ||
    (d.name as string) ||
    entry.entry_type
  )
}

function getSubtitle(entry: ContextEntryRow): string | null {
  const d = entry.data ?? {}
  return (d.role as string) || (d.degree as string) || (d.category as string) || null
}

function getDateRange(entry: ContextEntryRow): string | null {
  const d = entry.data ?? {}
  const start = (d.date_start ?? d.start_date ?? d.year) as string | undefined
  const end   = (d.date_end   ?? d.end_date)             as string | null | undefined

  if (!start) return null

  // end === null means explicitly cleared → "Present"
  // end === undefined means never set → also show "Present" for ongoing-type entries
  const endLabel = end ? end : 'Present'
  return `${start} – ${endLabel}`
}

function formatLabel(key: string): string {
  return key.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase())
}

// ─── Inline editable field ────────────────────────────────────────────────────

interface EditableFieldProps {
  label: string
  value: string | null | undefined
  fieldKey: string
  entryId: string
  userId: string
  onSave: () => void
}

function EditableField({
  label,
  value,
  fieldKey,
  entryId,
  userId,
  onSave,
}: EditableFieldProps) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(value ?? '')

  const handleSave = async () => {
    if (draft !== (value ?? '')) {
      try {
        await api.patchEntry(entryId, userId, { [fieldKey]: draft || null })
        onSave()
      } catch {
        // silently revert — field will show old value on next render
      }
    }
    setEditing(false)
  }

  if (editing) {
    return (
      <input
        type="text"
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={handleSave}
        onKeyDown={(e) => {
          if (e.key === 'Enter') handleSave()
          if (e.key === 'Escape') setEditing(false)
        }}
        className="text-xs border rounded px-1 py-0.5 bg-background w-full"
        autoFocus
      />
    )
  }

  return (
    <div className="group flex items-center gap-1">
      <span className="text-xs">
        {value || <em className="text-muted-foreground">not set</em>}
      </span>
      <button
        onClick={(e) => {
          e.stopPropagation()
          setEditing(true)
        }}
        className="opacity-0 group-hover:opacity-100 transition-opacity text-muted-foreground hover:text-foreground"
        title={`Edit ${label}`}
      >
        &#x270E;
      </button>
    </div>
  )
}

// ─── Inline date field with "Present" toggle ──────────────────────────────────

interface EditableDateFieldProps {
  label: string
  value: string | null | undefined
  fieldKey: string
  entryId: string
  userId: string
  /** true for date_end only — shows "Currently ongoing" checkbox */
  allowPresent: boolean
  onSave: () => void
}

function EditableDateField({
  label,
  value,
  fieldKey,
  entryId,
  userId,
  allowPresent,
  onSave,
}: EditableDateFieldProps) {
  const [editing, setEditing] = useState(false)
  const [isPresent, setIsPresent] = useState(!value)
  const [draft, setDraft] = useState(value ?? '')

  const handleSave = async () => {
    const patchVal = isPresent ? null : (draft || null)
    try {
      await api.patchEntry(entryId, userId, { [fieldKey]: patchVal })
      onSave()
    } catch {
      // silently revert
    }
    setEditing(false)
  }

  if (!editing) {
    return (
      <div className="group flex items-center gap-1">
        <span className="text-xs">
          {value
            ? value
            : allowPresent
            ? 'Present'
            : <em className="text-muted-foreground">not set</em>}
        </span>
        <button
          onClick={(e) => { e.stopPropagation(); setEditing(true) }}
          className="opacity-0 group-hover:opacity-100 transition-opacity text-muted-foreground hover:text-foreground"
          title={`Edit ${label}`}
        >
          &#x270E;
        </button>
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-1">
      {allowPresent && (
        <label className="flex items-center gap-1 text-xs cursor-pointer">
          <input
            type="checkbox"
            checked={isPresent}
            onChange={(e) => setIsPresent(e.target.checked)}
          />
          Currently ongoing (Present)
        </label>
      )}
      {!isPresent && (
        <input
          type="date"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') handleSave()
            if (e.key === 'Escape') setEditing(false)
          }}
          className="text-xs border rounded px-1 py-0.5 bg-background w-36"
          autoFocus={!allowPresent}
        />
      )}
      <div className="flex gap-1">
        <button
          onClick={handleSave}
          className="text-xs px-2 py-0.5 bg-primary text-primary-foreground rounded"
        >
          Save
        </button>
        <button
          onClick={() => setEditing(false)}
          className="text-xs px-2 py-0.5 border rounded"
        >
          Cancel
        </button>
      </div>
    </div>
  )
}

// ─── Expanded data section ────────────────────────────────────────────────────

function DataFields({ data }: { data: Record<string, unknown> }) {
  const skip = new Set([
    // shown in header
    'company', 'institution', 'name', 'role', 'degree', 'category',
    // legacy date field names
    'start_date', 'end_date', 'year',
    // actual schema date field names
    'date_start', 'date_end',
    // shown separately (EditableField) or via bullets section
    'bullets', 'contribution_type', 'entry_type',
    // shown via EditableField below
    'url', 'location',
  ])

  const entries = Object.entries(data).filter(
    ([k, v]) => !skip.has(k) && v !== null && v !== undefined && v !== ''
  )
  if (entries.length === 0) return null

  return (
    <div className="space-y-1">
      {entries.map(([k, v]) => (
        <div key={k} className="flex gap-2 text-xs">
          <span className="text-muted-foreground shrink-0 w-28 truncate">{formatLabel(k)}</span>
          <span className="text-foreground break-all">
            {TAG_ARRAY_KEYS.has(k) && Array.isArray(v) ? (
              <span className="flex flex-wrap gap-1">
                {(v as unknown[]).map((item, i) => (
                  <span
                    key={i}
                    className="inline-flex items-center px-1.5 py-0 rounded bg-muted text-muted-foreground"
                  >
                    {String(item)}
                  </span>
                ))}
              </span>
            ) : Array.isArray(v) ? (
              (v as unknown[]).map(String).join(', ')
            ) : k === 'url' && typeof v === 'string' ? (
              <a
                href={v}
                target="_blank"
                rel="noopener noreferrer"
                className="underline underline-offset-2 text-primary hover:text-primary/80"
                onClick={(e) => e.stopPropagation()}
              >
                {v}
              </a>
            ) : (
              String(v)
            )}
          </span>
        </div>
      ))}
    </div>
  )
}

// ─── Editable fields per entry_type ───────────────────────────────────────────

/** Returns the list of inline-editable field specs for a given entry_type. */
function editableFieldsFor(
  entryType: string
): Array<{ key: string; label: string }> {
  const dateFields = [
    { key: 'date_start', label: 'Start Date' },
    { key: 'date_end', label: 'End Date' },
  ]

  switch (entryType) {
    case 'experience':
      return [
        ...dateFields,
        { key: 'location', label: 'Location' },
        { key: 'role', label: 'Role' },
        { key: 'company', label: 'Company' },
      ]
    case 'project':
      return [
        ...dateFields,
        { key: 'url', label: 'URL' },
      ]
    case 'education':
      return [...dateFields]
    case 'open_source':
      return [
        ...dateFields,
        { key: 'url', label: 'URL' },
      ]
    case 'extracurricular':
      return [
        ...dateFields,
        { key: 'role', label: 'Role' },
      ]
    case 'publication':
      return [{ key: 'url', label: 'URL' }]
    case 'certification':
      return [{ key: 'url', label: 'URL' }]
    default:
      return []
  }
}

// ─── Main component ──────────────────────────────────────────────────────────

interface ContextEntryCardProps {
  entry: ContextEntryRow
  isExpanded: boolean
  onToggle: () => void
  /** Called after a successful inline edit so the parent can re-fetch. */
  onRefresh?: () => void
}

export default function ContextEntryCard({
  entry,
  isExpanded,
  onToggle,
  onRefresh,
}: ContextEntryCardProps) {
  const [evergreen, setEvergreen] = useState(entry.flagged_evergreen)
  const [evergreenLoading, setEvergreenLoading] = useState(false)

  const title = getTitle(entry)
  const subtitle = getSubtitle(entry)
  const dateRange = getDateRange(entry)
  const qualityPct = Math.round(entry.quality_score * 100)

  // Only skill / certification entries show the evergreen toggle.
  const showEvergreenToggle =
    entry.entry_type === 'skill' || entry.entry_type === 'certification'

  const bullets: BulletItem[] = Array.isArray(entry.data?.bullets)
    ? normalizeBullets(entry.data!.bullets as unknown[])
    : []

  const visibleTags = entry.tags.slice(0, MAX_COLLAPSED_TAGS)
  const hiddenTagCount = Math.max(0, entry.tags.length - MAX_COLLAPSED_TAGS)

  const editableFields = editableFieldsFor(entry.entry_type)

  async function handleEvergreenToggle(e: React.MouseEvent) {
    e.stopPropagation()
    if (evergreenLoading) return
    const newValue = !evergreen
    setEvergreen(newValue) // optimistic
    setEvergreenLoading(true)
    try {
      await api.toggleEvergreen(entry.entry_id, MVP_USER_ID, newValue)
    } catch {
      setEvergreen(!newValue) // rollback
    } finally {
      setEvergreenLoading(false)
    }
  }

  function handleFieldSave() {
    if (onRefresh) {
      onRefresh()
    }
  }

  return (
    <div
      role="button"
      tabIndex={0}
      aria-expanded={isExpanded}
      onClick={onToggle}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onToggle()
        }
      }}
      className="border rounded-lg p-4 hover:bg-muted/40 transition-colors cursor-pointer space-y-3 select-none"
    >
      {/* ── Collapsed header ─────────────────────────────────────────── */}
      <div className="flex items-start gap-3">
        {/* Type badge */}
        <span
          className={`inline-flex items-center px-2 py-0.5 rounded text-xs font-medium shrink-0 capitalize ${typeBadgeClass(entry.entry_type)}`}
        >
          {entry.entry_type.replace('_', ' ')}
        </span>

        {/* Title + subtitle + date */}
        <div className="flex-1 min-w-0">
          <p className="text-sm font-semibold leading-snug truncate">{title}</p>
          {subtitle && (
            <p className="text-xs text-muted-foreground truncate">{subtitle}</p>
          )}
          {dateRange && (
            <p className="text-xs text-muted-foreground">{dateRange}</p>
          )}
        </div>

        {/* Right-side controls */}
        <div className="flex items-center gap-2 shrink-0" onClick={(e) => e.stopPropagation()}>
          {/* Evergreen toggle — only for skill/certification */}
          {showEvergreenToggle && (
            <Button
              variant="ghost"
              size="sm"
              className={`h-7 px-2 text-xs ${
                evergreen
                  ? 'text-amber-500 dark:text-amber-400'
                  : 'text-muted-foreground'
              }`}
              title={evergreen ? 'Evergreen (click to unmark)' : 'Mark as evergreen'}
              onClick={handleEvergreenToggle}
              disabled={evergreenLoading}
              aria-pressed={evergreen}
            >
              {/* Lightning bolt icon rendered as text */}
              <span aria-hidden>&#x26A1;</span>
            </Button>
          )}

          {/* Expand/collapse chevron */}
          <span className="text-muted-foreground text-xs" aria-hidden>
            {isExpanded ? '▲' : '▼'}
          </span>
        </div>
      </div>

      {/* ── Quality bar ───────────────────────────────────────────────── */}
      <div className="space-y-1">
        <div className="flex justify-between items-center text-xs text-muted-foreground">
          <span>Quality</span>
          <span>{qualityPct}%</span>
        </div>
        <div className="w-full h-1.5 bg-muted rounded-full overflow-hidden">
          <div
            className={`h-full rounded-full transition-all duration-300 ${qualityBarColor(entry.quality_score)}`}
            style={{ width: `${qualityPct}%` }}
          />
        </div>
      </div>

      {/* ── Tags + contribution (collapsed) ───────────────────────────── */}
      <div className="flex flex-wrap gap-1.5 items-center">
        {visibleTags.map((tag) => (
          <Badge key={tag} variant="secondary" className="text-xs px-1.5 py-0">
            {tag}
          </Badge>
        ))}
        {hiddenTagCount > 0 && (
          <span className="text-xs text-muted-foreground">+{hiddenTagCount} more</span>
        )}
        {entry.contribution_type && (
          <Badge variant="outline" className="text-xs px-1.5 py-0 ml-auto capitalize">
            {entry.contribution_type.replace('_', ' ')}
          </Badge>
        )}
      </div>

      {/* ── Expanded detail ────────────────────────────────────────────── */}
      {isExpanded && (
        <div
          className="border-t pt-3 space-y-4"
          onClick={(e) => e.stopPropagation()}
        >
          {/* Score row */}
          <div className="flex gap-4 text-xs">
            <div>
              <span className="text-muted-foreground">Impact </span>
              <span className="font-medium tabular-nums">
                {entry.impact_score.toFixed(2)}
              </span>
            </div>
            <div>
              <span className="text-muted-foreground">Recency </span>
              <span className="font-medium tabular-nums">
                {entry.recency_score.toFixed(2)}
              </span>
            </div>
            <div>
              <span className="text-muted-foreground">Version </span>
              <span className="font-medium tabular-nums">v{entry.version}</span>
            </div>
          </div>

          {/* Quality flags */}
          {entry.quality_flags.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {entry.quality_flags.map((flag) => (
                <span
                  key={flag}
                  className="inline-flex items-center px-2 py-0.5 rounded text-xs bg-amber-100 text-amber-800 dark:bg-amber-900/40 dark:text-amber-300"
                >
                  {flag.replace(/_/g, ' ')}
                </span>
              ))}
            </div>
          )}

          {/* Inline editable fields (per entry_type) */}
          {editableFields.length > 0 && (
            <div className="space-y-1.5">
              {editableFields.map(({ key, label }) => {
                const isDateField = key === 'date_start' || key === 'date_end'
                const fieldValue =
                  ((entry.data ?? {})[key] as string | null | undefined) ??
                  (key === 'date_start'
                    ? ((entry.data ?? {})['start_date'] as string | null | undefined)
                    : key === 'date_end'
                    ? ((entry.data ?? {})['end_date'] as string | null | undefined)
                    : undefined)

                return (
                  <div key={key} className="flex gap-2 text-xs items-center">
                    <span className="text-muted-foreground shrink-0 w-28 truncate">
                      {label}
                    </span>
                    {isDateField ? (
                      <EditableDateField
                        label={label}
                        value={fieldValue}
                        fieldKey={key}
                        entryId={entry.entry_id}
                        userId={MVP_USER_ID}
                        allowPresent={key === 'date_end'}
                        onSave={handleFieldSave}
                      />
                    ) : (
                      <EditableField
                        label={label}
                        value={fieldValue}
                        fieldKey={key}
                        entryId={entry.entry_id}
                        userId={MVP_USER_ID}
                        onSave={handleFieldSave}
                      />
                    )}
                  </div>
                )
              })}
            </div>
          )}

          {/* Bullets — copy-protected */}
          {bullets.length > 0 && (
            <ul
              className="space-y-1 list-disc list-inside select-none"
              onCopy={(e) => e.preventDefault()}
            >
              {bullets.map((b, i) => (
                <li key={i} className="text-xs text-foreground leading-snug">
                  {b.text}
                  {b.impact_markers && b.impact_markers.length > 0 && (
                    <span className="ml-1 text-muted-foreground">
                      ({b.impact_markers.join(', ')})
                    </span>
                  )}
                  {b.confidence_marker && (
                    <span className="ml-1 text-amber-500 text-[10px] font-medium">
                      {b.confidence_marker}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          )}

          {/* All tags */}
          {entry.tags.length > MAX_COLLAPSED_TAGS && (
            <div className="flex flex-wrap gap-1.5">
              {entry.tags.map((tag) => (
                <Badge key={tag} variant="secondary" className="text-xs px-1.5 py-0">
                  {tag}
                </Badge>
              ))}
            </div>
          )}

          {/* Structured data fields (remaining, not already shown inline) */}
          <DataFields data={entry.data ?? {}} />
        </div>
      )}
    </div>
  )
}
