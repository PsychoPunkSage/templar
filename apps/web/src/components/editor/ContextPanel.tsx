'use client'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'

const API_BASE = process.env.NEXT_PUBLIC_API_URL ?? 'http://localhost:8080'
const MVP_USER_ID = '00000000-0000-0000-0000-000000000001'

export function ContextPanel() {
  const [rawText, setRawText] = useState('')
  const [status, setStatus] = useState<'idle' | 'loading' | 'done' | 'error'>('idle')
  const [message, setMessage] = useState('')

  const handleIngest = async () => {
    if (!rawText.trim()) return
    setStatus('loading')
    try {
      // Step 1: ingest (returns pending items needing confirmation)
      const res = await fetch(`${API_BASE}/api/v1/context/ingest`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ user_id: MVP_USER_ID, raw_text: rawText }),
      })
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      const data = await res.json()

      // Step 2: auto-confirm all pending items
      if (data.pending_items?.length > 0) {
        const confirmRes = await fetch(`${API_BASE}/api/v1/context/ingest/confirm`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            user_id: MVP_USER_ID,
            confirmed_items: data.pending_items,
          }),
        })
        if (!confirmRes.ok) throw new Error(`Confirm HTTP ${confirmRes.status}`)
      }

      setStatus('done')
      setMessage(`Context ingested successfully.`)
      setRawText('')
    } catch (e) {
      setStatus('error')
      setMessage(e instanceof Error ? e.message : 'Ingestion failed')
    }
  }

  return (
    <div className="flex flex-col gap-2">
      <label className="text-sm font-medium text-muted-foreground">
        Add Context (resume, experience, projects)
      </label>
      <Textarea
        value={rawText}
        onChange={(e) => setRawText(e.target.value)}
        placeholder="Paste your resume, work experience, or project descriptions here..."
        className="h-36 resize-none font-mono text-sm"
      />
      <Button
        size="sm"
        variant="outline"
        onClick={handleIngest}
        disabled={status === 'loading' || !rawText.trim()}
      >
        {status === 'loading' ? 'Ingesting...' : 'Add to Context'}
      </Button>
      {message && (
        <p className={`text-xs ${status === 'error' ? 'text-destructive' : 'text-green-600'}`}>
          {message}
        </p>
      )}
    </div>
  )
}
