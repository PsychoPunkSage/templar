'use client'

import { useCallback, useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'

const API_BASE = process.env.NEXT_PUBLIC_API_URL ?? 'http://localhost:8080'
const MVP_USER_ID = '00000000-0000-0000-0000-000000000001'

// ─── Types ─────────────────────────────────────────────────────────────────

interface BatchItemStatus {
  id: string
  entry_index: number
  status: 'pending' | 'processing' | 'succeeded' | 'failed'
  error_msg: string | null
  entry_id: string | null
}

interface BatchStatusResponse {
  batch_id: string
  source_type: string
  filename: string | null
  total: number
  pending: number
  succeeded: number
  failed: number
  status: 'queued' | 'processing' | 'done'
  items: BatchItemStatus[]
}

// ─── Progress sub-component ────────────────────────────────────────────────

function BatchProgressView({ batchStatus }: { batchStatus: BatchStatusResponse }) {
  const processed = batchStatus.succeeded + batchStatus.failed
  const progressPercent =
    batchStatus.total > 0 ? (processed / batchStatus.total) * 100 : 0

  const statusColor =
    batchStatus.status === 'done'
      ? batchStatus.failed > 0
        ? 'text-yellow-600 dark:text-yellow-400'
        : 'text-green-600 dark:text-green-400'
      : 'text-muted-foreground'

  return (
    <div className="space-y-2 mt-3">
      {/* Summary row */}
      <div className="flex justify-between items-center text-xs">
        <span className="text-muted-foreground">
          {processed}/{batchStatus.total} entries processed
          {batchStatus.failed > 0 && (
            <span className="text-destructive ml-1">({batchStatus.failed} failed)</span>
          )}
        </span>
        <span className={`capitalize font-medium ${statusColor}`}>
          {batchStatus.status}
        </span>
      </div>

      {/* Progress bar */}
      <div className="w-full h-1.5 bg-muted rounded-full overflow-hidden">
        <div
          className={`h-full rounded-full transition-all duration-300 ${
            batchStatus.status === 'done' && batchStatus.failed === 0
              ? 'bg-green-500'
              : batchStatus.failed > 0
              ? 'bg-yellow-500'
              : 'bg-primary'
          }`}
          style={{ width: `${progressPercent}%` }}
        />
      </div>

      {/* Per-item list */}
      <div className="max-h-40 overflow-y-auto space-y-1">
        {batchStatus.items.map((item) => (
          <div
            key={item.id}
            className={`text-xs px-2 py-1 rounded flex items-start gap-2 ${
              item.status === 'succeeded'
                ? 'bg-green-50 dark:bg-green-950/30 text-green-700 dark:text-green-300'
                : item.status === 'failed'
                ? 'bg-red-50 dark:bg-red-950/30 text-destructive'
                : 'bg-muted text-muted-foreground'
            }`}
          >
            <span className="shrink-0 font-mono">#{item.entry_index + 1}</span>
            <span className="capitalize">{item.status}</span>
            {item.error_msg && (
              <span className="ml-1 opacity-80 break-all">{item.error_msg}</span>
            )}
          </div>
        ))}
      </div>
    </div>
  )
}

// ─── Main component ────────────────────────────────────────────────────────

export function ContextPanel({ onContextUpdated }: { onContextUpdated?: () => void }) {
  const [activeTab, setActiveTab] = useState<'text' | 'file'>('text')

  // Text tab state
  const [rawText, setRawText] = useState('')

  // File tab state
  const [selectedFile, setSelectedFile] = useState<File | null>(null)
  const [isDragging, setIsDragging] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)

  // Shared submission state
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [submitError, setSubmitError] = useState<string | null>(null)

  // Batch polling state
  const [batchId, setBatchId] = useState<string | null>(null)
  const [batchStatus, setBatchStatus] = useState<BatchStatusResponse | null>(null)
  const pollIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null)

  // ── Polling effect ──────────────────────────────────────────────────────

  const stopPolling = useCallback(() => {
    if (pollIntervalRef.current !== null) {
      clearInterval(pollIntervalRef.current)
      pollIntervalRef.current = null
    }
  }, [])

  useEffect(() => {
    if (!batchId) return

    const poll = async () => {
      try {
        const res = await fetch(
          `${API_BASE}/api/v1/context/ingest/batch/${batchId}`
        )
        if (!res.ok) return
        const data: BatchStatusResponse = await res.json()
        setBatchStatus(data)
        if (data.status === 'done') {
          stopPolling()
          onContextUpdated?.()
        }
      } catch {
        // Polling errors are silent — the user sees the last known status
      }
    }

    // Poll immediately then every 2 seconds
    poll()
    pollIntervalRef.current = setInterval(poll, 2000)

    return () => stopPolling()
  }, [batchId, stopPolling, onContextUpdated])

  // ── Text tab submission ─────────────────────────────────────────────────

  const handleTextSubmit = async () => {
    if (!rawText.trim()) return
    setIsSubmitting(true)
    setSubmitError(null)
    setBatchId(null)
    setBatchStatus(null)

    try {
      const res = await fetch(`${API_BASE}/api/v1/context/ingest/batch`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ user_id: MVP_USER_ID, raw_text: rawText }),
      })
      if (!res.ok) {
        const errData = await res.json().catch(() => ({}))
        throw new Error(errData?.error?.message ?? `HTTP ${res.status}`)
      }
      const data = await res.json()
      setBatchId(data.batch_id)
      setRawText('')
    } catch (e) {
      setSubmitError(e instanceof Error ? e.message : 'Submission failed')
    } finally {
      setIsSubmitting(false)
    }
  }

  // ── File tab handlers ───────────────────────────────────────────────────

  const handleFileChange = (file: File | null) => {
    if (!file) return
    setSelectedFile(file)
    setSubmitError(null)
  }

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault()
    setIsDragging(false)
    const file = e.dataTransfer.files?.[0] ?? null
    handleFileChange(file)
  }, []) // eslint-disable-line react-hooks/exhaustive-deps

  const handleFileSubmit = async () => {
    if (!selectedFile) return
    setIsSubmitting(true)
    setSubmitError(null)
    setBatchId(null)
    setBatchStatus(null)

    try {
      const formData = new FormData()
      formData.append('user_id', MVP_USER_ID)
      formData.append('file', selectedFile)

      const res = await fetch(`${API_BASE}/api/v1/context/ingest/upload`, {
        method: 'POST',
        body: formData,
      })
      if (!res.ok) {
        const errData = await res.json().catch(() => ({}))
        throw new Error(errData?.error?.message ?? `HTTP ${res.status}`)
      }
      const data = await res.json()
      setBatchId(data.batch_id)
      setSelectedFile(null)
      if (fileInputRef.current) fileInputRef.current.value = ''
    } catch (e) {
      setSubmitError(e instanceof Error ? e.message : 'Upload failed')
    } finally {
      setIsSubmitting(false)
    }
  }

  // ── Render ──────────────────────────────────────────────────────────────

  return (
    <div className="flex flex-col gap-2">
      <label className="text-sm font-medium text-muted-foreground">
        Add Context (resume, experience, projects)
      </label>

      <Tabs
        value={activeTab}
        onValueChange={(v) => setActiveTab(v as 'text' | 'file')}
      >
        <TabsList className="w-full">
          <TabsTrigger value="text" className="flex-1">
            Paste Text
          </TabsTrigger>
          <TabsTrigger value="file" className="flex-1">
            Upload File
          </TabsTrigger>
        </TabsList>

        {/* ── Paste Text tab ─────────────────────────────────────────── */}
        <TabsContent value="text" className="mt-2 space-y-2">
          <Textarea
            value={rawText}
            onChange={(e) => setRawText(e.target.value)}
            placeholder={
              'Paste your resume, work experience, or project descriptions here...\n\nSeparate multiple entries with a line containing only ---'
            }
            className="h-36 resize-none font-mono text-sm"
          />
          <Button
            size="sm"
            variant="outline"
            onClick={handleTextSubmit}
            disabled={isSubmitting || !rawText.trim()}
            className="w-full"
          >
            {isSubmitting ? 'Submitting...' : 'Add to Context'}
          </Button>
        </TabsContent>

        {/* ── Upload File tab ────────────────────────────────────────── */}
        <TabsContent value="file" className="mt-2 space-y-2">
          {/* Drop zone */}
          <div
            role="button"
            tabIndex={0}
            aria-label="Upload file drop zone"
            className={`border-2 border-dashed rounded-lg p-5 text-center cursor-pointer transition-colors ${
              isDragging
                ? 'border-primary bg-primary/5'
                : 'border-border hover:border-primary/60 hover:bg-muted/50'
            }`}
            onClick={() => fileInputRef.current?.click()}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') fileInputRef.current?.click()
            }}
            onDrop={handleDrop}
            onDragOver={(e) => {
              e.preventDefault()
              setIsDragging(true)
            }}
            onDragLeave={() => setIsDragging(false)}
          >
            <input
              ref={fileInputRef}
              type="file"
              accept=".md,.txt,.pdf"
              className="hidden"
              onChange={(e) => handleFileChange(e.target.files?.[0] ?? null)}
            />
            {selectedFile ? (
              <div className="space-y-1">
                <p className="text-sm font-medium">{selectedFile.name}</p>
                <p className="text-xs text-muted-foreground">
                  {(selectedFile.size / 1024).toFixed(1)} KB
                </p>
              </div>
            ) : (
              <div className="space-y-1">
                <p className="text-sm text-muted-foreground">
                  Drop a file here, or click to browse
                </p>
                <p className="text-xs text-muted-foreground">
                  .md, .txt, .pdf — max 10 MB
                </p>
                <p className="text-xs text-muted-foreground">
                  Multi-entry documents split automatically on <code className="font-mono">---</code>
                </p>
              </div>
            )}
          </div>

          <Button
            size="sm"
            variant="outline"
            onClick={handleFileSubmit}
            disabled={isSubmitting || !selectedFile}
            className="w-full"
          >
            {isSubmitting ? 'Uploading...' : 'Upload & Ingest'}
          </Button>
        </TabsContent>
      </Tabs>

      {/* ── Error message ─────────────────────────────────────────────── */}
      {submitError && (
        <p className="text-xs text-destructive">{submitError}</p>
      )}

      {/* ── Batch progress (shown after submission, for both tabs) ─────── */}
      {batchStatus && <BatchProgressView batchStatus={batchStatus} />}
    </div>
  )
}
