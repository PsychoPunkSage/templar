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

interface UploadEntry {
  id: string
  file: File
  uploadStatus: 'pending' | 'uploading' | 'done' | 'error'
  uploadError: string | null
  batchId: string | null
  batchStatus: BatchStatusResponse | null
}

// ─── Progress sub-components ────────────────────────────────────────────────

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
    <div className="space-y-2">
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

function IndeterminateBar() {
  return (
    <div className="w-full h-1.5 bg-muted rounded-full overflow-hidden">
      <div className="h-full w-1/2 bg-primary rounded-full animate-pulse" />
    </div>
  )
}

function StatusBadge({ status }: { status: UploadEntry['uploadStatus'] }) {
  const cls =
    status === 'done'
      ? 'text-green-600 dark:text-green-400'
      : status === 'error'
      ? 'text-destructive'
      : status === 'uploading'
      ? 'text-primary'
      : 'text-muted-foreground'
  return <span className={`shrink-0 capitalize ${cls}`}>{status}</span>
}

// ─── Main component ────────────────────────────────────────────────────────

export function ContextPanel({ onContextUpdated }: { onContextUpdated?: () => void }) {
  const [activeTab, setActiveTab] = useState<'text' | 'file'>('text')

  // Text tab state
  const [rawText, setRawText] = useState('')
  const [textSubmitError, setTextSubmitError] = useState<string | null>(null)
  const [textBatchId, setTextBatchId] = useState<string | null>(null)
  const [textBatchStatus, setTextBatchStatus] = useState<BatchStatusResponse | null>(null)
  const textPollRef = useRef<ReturnType<typeof setInterval> | null>(null)

  // File tab state
  const [selectedFiles, setSelectedFiles] = useState<File[]>([])
  const [isDragging, setIsDragging] = useState(false)
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [uploadEntries, setUploadEntries] = useState<UploadEntry[]>([])
  const fileInputRef = useRef<HTMLInputElement>(null)

  // ── Text tab polling ──────────────────────────────────────────────────────

  const stopTextPoll = useCallback(() => {
    if (textPollRef.current !== null) {
      clearInterval(textPollRef.current)
      textPollRef.current = null
    }
  }, [])

  useEffect(() => {
    if (!textBatchId) return

    const poll = async () => {
      try {
        const res = await fetch(`${API_BASE}/api/v1/context/ingest/batch/${textBatchId}`)
        if (!res.ok) return
        const data: BatchStatusResponse = await res.json()
        setTextBatchStatus(data)
        if (data.status === 'done') {
          stopTextPoll()
          onContextUpdated?.()
        }
      } catch {
        // silent
      }
    }

    poll()
    textPollRef.current = setInterval(poll, 2000)
    return () => stopTextPoll()
  }, [textBatchId, stopTextPoll, onContextUpdated])

  // ── File upload polling ───────────────────────────────────────────────────

  useEffect(() => {
    const active = uploadEntries.filter(
      (e) => e.batchId !== null && e.batchStatus?.status !== 'done'
    )
    if (active.length === 0) return

    const poll = async () => {
      for (const entry of active) {
        if (!entry.batchId) continue
        try {
          const res = await fetch(`${API_BASE}/api/v1/context/ingest/batch/${entry.batchId}`)
          if (!res.ok) continue
          const data: BatchStatusResponse = await res.json()
          setUploadEntries((prev) =>
            prev.map((e) => (e.id === entry.id ? { ...e, batchStatus: data } : e))
          )
          if (data.status === 'done') onContextUpdated?.()
        } catch {
          // silent
        }
      }
    }

    poll()
    const interval = setInterval(poll, 2000)
    return () => clearInterval(interval)
  }, [uploadEntries, onContextUpdated])

  // ── Text tab submission ───────────────────────────────────────────────────

  const handleTextSubmit = async () => {
    if (!rawText.trim()) return
    setIsSubmitting(true)
    setTextSubmitError(null)
    setTextBatchId(null)
    setTextBatchStatus(null)

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
      setTextBatchId(data.batch_id)
      setRawText('')
    } catch (e) {
      setTextSubmitError(e instanceof Error ? e.message : 'Submission failed')
    } finally {
      setIsSubmitting(false)
    }
  }

  // ── File tab handlers ─────────────────────────────────────────────────────

  const handleFilesChange = useCallback((list: FileList | null) => {
    if (!list || list.length === 0) return
    setSelectedFiles(Array.from(list))
    setUploadEntries([])
  }, [])

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault()
      setIsDragging(false)
      handleFilesChange(e.dataTransfer.files)
    },
    [handleFilesChange]
  )

  const handleFileSubmit = async () => {
    if (selectedFiles.length === 0) return
    setIsSubmitting(true)

    const entries: UploadEntry[] = selectedFiles.map((f) => ({
      id: crypto.randomUUID(),
      file: f,
      uploadStatus: 'pending',
      uploadError: null,
      batchId: null,
      batchStatus: null,
    }))
    setUploadEntries(entries)

    for (let i = 0; i < entries.length; i++) {
      setUploadEntries((prev) =>
        prev.map((e, idx) => (idx === i ? { ...e, uploadStatus: 'uploading' } : e))
      )
      try {
        const fd = new FormData()
        fd.append('user_id', MVP_USER_ID)
        fd.append('file', entries[i].file)
        const res = await fetch(`${API_BASE}/api/v1/context/ingest/upload`, {
          method: 'POST',
          body: fd,
        })
        if (!res.ok) {
          const errData = await res.json().catch(() => ({}))
          throw new Error(errData?.error?.message ?? `HTTP ${res.status}`)
        }
        const data = await res.json()
        setUploadEntries((prev) =>
          prev.map((e, idx) =>
            idx === i ? { ...e, uploadStatus: 'done', batchId: data.batch_id } : e
          )
        )
      } catch (err) {
        setUploadEntries((prev) =>
          prev.map((e, idx) =>
            idx === i ? { ...e, uploadStatus: 'error', uploadError: String(err) } : e
          )
        )
      }
    }

    setSelectedFiles([])
    if (fileInputRef.current) fileInputRef.current.value = ''
    setIsSubmitting(false)
  }

  // ── Render ────────────────────────────────────────────────────────────────

  const uploadingIdx = uploadEntries.findIndex((e) => e.uploadStatus === 'uploading')

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
          {textSubmitError && (
            <p className="text-xs text-destructive">{textSubmitError}</p>
          )}
          {textBatchStatus && <BatchProgressView batchStatus={textBatchStatus} />}
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
              multiple
              className="hidden"
              onChange={(e) => handleFilesChange(e.target.files)}
            />
            {selectedFiles.length > 0 ? (
              <div className="space-y-1 text-left">
                {selectedFiles.map((f, i) => (
                  <div key={i} className="flex justify-between items-center text-sm">
                    <span className="font-medium truncate">{f.name}</span>
                    <span className="text-xs text-muted-foreground ml-2 shrink-0">
                      {(f.size / 1024).toFixed(1)} KB
                    </span>
                  </div>
                ))}
              </div>
            ) : (
              <div className="space-y-1">
                <p className="text-sm text-muted-foreground">
                  Drop files here, or click to browse
                </p>
                <p className="text-xs text-muted-foreground">
                  .md, .txt, .pdf — max 10 MB each
                </p>
                <p className="text-xs text-muted-foreground">
                  Multi-entry documents split automatically on{' '}
                  <code className="font-mono">---</code>
                </p>
              </div>
            )}
          </div>

          <Button
            size="sm"
            variant="outline"
            onClick={handleFileSubmit}
            disabled={isSubmitting || selectedFiles.length === 0}
            className="w-full"
          >
            {isSubmitting
              ? `Uploading ${uploadingIdx >= 0 ? `${uploadingIdx + 1}/${uploadEntries.length}` : ''}...`
              : `Upload & Ingest${selectedFiles.length > 1 ? ` (${selectedFiles.length} files)` : ''}`}
          </Button>

          {/* Per-file upload cards */}
          {uploadEntries.length > 0 && (
            <div className="space-y-2">
              {uploadEntries.map((entry) => (
                <div key={entry.id} className="rounded-lg border p-2 space-y-1.5">
                  <div className="flex items-center justify-between text-xs">
                    <span className="font-medium truncate">{entry.file.name}</span>
                    <StatusBadge status={entry.uploadStatus} />
                  </div>
                  {entry.uploadStatus === 'uploading' && <IndeterminateBar />}
                  {entry.batchStatus && (
                    <BatchProgressView batchStatus={entry.batchStatus} />
                  )}
                  {entry.uploadError && (
                    <p className="text-xs text-destructive">{entry.uploadError}</p>
                  )}
                </div>
              ))}
            </div>
          )}
        </TabsContent>
      </Tabs>
    </div>
  )
}
