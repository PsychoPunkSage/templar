'use client'

// Standalone context management page.
// Left panel: ingestion (ContextPanel — paste text or upload file).
// Right panel: Context Library — browse, inspect and manage ingested entries.
//
// Desktop (lg+): viewport-locked layout — header is fixed, left panel stays
// visible while right column scrolls independently.
// Mobile: normal stacked page scroll (no dual-scroll UX on small screens).

import { useState } from 'react'
import { ContextPanel } from '@/components/editor/ContextPanel'
import ContextLibrary from '@/components/context/ContextLibrary'

export default function ContextPage() {
  // Incrementing this key triggers ContextLibrary to re-fetch entries after
  // a successful ingestion batch completes.
  const [refreshKey, setRefreshKey] = useState(0)

  return (
    // lg+: locked to viewport height (53px = global nav, matches layout.tsx)
    // mobile: normal flex-col, page-level scroll
    <div className="flex flex-col lg:h-[calc(100vh-53px)]">

      {/* Page header — always visible, never scrolls */}
      <div className="shrink-0 px-6 pt-8 pb-4 max-w-6xl mx-auto w-full">
        <h1 className="text-2xl font-bold tracking-tight">Context</h1>
        <p className="text-sm text-muted-foreground mt-1">
          Your professional context — experience, projects, education, skills.
          Every resume bullet is grounded to an entry here.
        </p>
      </div>

      {/* Two-column area */}
      {/* lg+: flex-row, flex-1 fills remaining height, overflow-hidden clips columns */}
      {/* mobile: flex-col, normal stacking */}
      <div className="flex flex-col lg:flex-row lg:flex-1 lg:overflow-hidden px-6 pb-6 gap-8 max-w-6xl mx-auto w-full">

        {/* Left: ingestion panel — fixed width, stays in place on desktop */}
        <div className="w-full lg:w-80 shrink-0">
          <ContextPanel onContextUpdated={() => setRefreshKey((k) => k + 1)} />
        </div>

        {/* Right: library — scrolls independently on desktop */}
        <div className="flex-1 min-w-0 lg:overflow-y-auto">
          <ContextLibrary refreshKey={refreshKey} />
        </div>

      </div>
    </div>
  )
}
