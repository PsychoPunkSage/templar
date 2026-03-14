'use client'

// Standalone context management page.
// Left panel: ingestion (ContextPanel — paste text or upload file).
// Right panel: Context Library — browse, inspect and manage ingested entries.

import { useState } from 'react'
import { ContextPanel } from '@/components/editor/ContextPanel'
import ContextLibrary from '@/components/context/ContextLibrary'

export default function ContextPage() {
  // Incrementing this key triggers ContextLibrary to re-fetch entries after
  // a successful ingestion batch completes.
  const [refreshKey, setRefreshKey] = useState(0)

  return (
    <div className="max-w-6xl mx-auto px-6 py-10">
      <div className="mb-8">
        <h1 className="text-2xl font-bold tracking-tight">Context</h1>
        <p className="text-sm text-muted-foreground mt-1">
          Your professional context — experience, projects, education, skills.
          Every resume bullet is grounded to an entry here.
        </p>
      </div>

      <div className="flex flex-col lg:flex-row gap-8 items-start">
        {/* Left: ingestion panel (fixed width) */}
        <div className="w-full lg:w-80 shrink-0">
          <ContextPanel onContextUpdated={() => setRefreshKey((k) => k + 1)} />
        </div>

        {/* Right: library (fills remaining space) */}
        <div className="flex-1 min-w-0">
          <ContextLibrary refreshKey={refreshKey} />
        </div>
      </div>
    </div>
  )
}
