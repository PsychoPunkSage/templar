"use client";

import { Loader2, RefreshCw, Copy, Check, AlertTriangle } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useCoverLetterStore, CoverLetterTone, CoverLetterFocus } from "@/store/coverLetterStore";

interface CoverLetterPaneProps {
  jdText: string;
  resumeId: string | null;
  personaId: string | null;
}

const ROLE_LABELS: Record<string, string> = {
  hook: "Opening",
  fit: "Why I'm a Fit",
  culture: "Culture & Values",
  close: "Closing",
};

export function CoverLetterPane({ jdText, resumeId, personaId }: CoverLetterPaneProps) {
  const {
    coverId,
    content,
    companyName,
    roleTitle,
    status,
    tone,
    focus,
    generatedWithJdText,
    error,
    generate,
    setTone,
    setFocus,
    clearError,
  } = useCoverLetterStore();

  const [copied, setCopied] = useState(false);

  const isStale =
    !!generatedWithJdText &&
    !!coverId &&
    generatedWithJdText !== jdText;

  const handleGenerate = () => {
    generate(jdText, resumeId, personaId);
  };

  const handleCopy = async () => {
    if (!content) return;
    const full = content.map((p) => p.text).join("\n\n");
    await navigator.clipboard.writeText(full);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  // ── Generating state ──────────────────────────────────────────────────────
  if (status === "generating") {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="flex flex-col items-center gap-3 text-muted-foreground">
          <Loader2 className="h-6 w-6 animate-spin" />
          <span className="text-sm">Writing cover letter...</span>
        </div>
      </div>
    );
  }

  // ── Done state — show paragraphs ──────────────────────────────────────────
  if (status === "done" && content) {
    return (
      <div className="flex flex-col h-full">
        {/* Header */}
        <div className="px-5 py-3 border-b shrink-0">
          <div className="flex items-start justify-between gap-2">
            <div className="min-w-0">
              {(companyName || roleTitle) && (
                <p className="text-xs text-muted-foreground truncate">
                  {[roleTitle, companyName].filter(Boolean).join(" · ")}
                </p>
              )}
            </div>
            <div className="flex items-center gap-1.5 shrink-0">
              {isStale && (
                <span className="flex items-center gap-1 text-xs text-amber-600 dark:text-amber-400">
                  <AlertTriangle className="h-3 w-3" />
                  JD changed
                </span>
              )}
              <Button
                variant="ghost"
                size="sm"
                onClick={handleGenerate}
                disabled={!jdText.trim()}
                className="h-7 gap-1.5 text-xs"
              >
                <RefreshCw className="h-3 w-3" />
                Regenerate
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={handleCopy}
                className="h-7 gap-1.5 text-xs"
              >
                {copied ? (
                  <Check className="h-3 w-3 text-green-500" />
                ) : (
                  <Copy className="h-3 w-3" />
                )}
                {copied ? "Copied" : "Copy"}
              </Button>
            </div>
          </div>
        </div>

        {/* Paragraphs */}
        <ScrollArea className="flex-1">
          <div className="px-5 py-4 space-y-5">
            {content.map((para) => (
              <div key={para.role}>
                <p className="text-[10px] uppercase tracking-widest text-muted-foreground mb-1.5 font-medium">
                  {ROLE_LABELS[para.role] ?? para.role}
                </p>
                <p className="text-sm leading-relaxed">{para.text}</p>
              </div>
            ))}
          </div>
        </ScrollArea>
      </div>
    );
  }

  // ── Idle / failed state — controls + generate button ─────────────────────
  return (
    <div className="flex flex-col h-full">
      <ScrollArea className="flex-1">
        <div className="flex flex-col items-center justify-center min-h-[320px] px-6 py-8 gap-6">
          {/* Error */}
          {status === "failed" && error && (
            <div className="w-full max-w-sm bg-destructive/10 text-destructive text-sm rounded-md px-3 py-2 flex items-start gap-2">
              <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" />
              <span>{error}</span>
              <button onClick={clearError} className="ml-auto text-xs opacity-70 hover:opacity-100">✕</button>
            </div>
          )}

          {/* Controls */}
          <div className="w-full max-w-sm space-y-4">
            {/* Tone */}
            <div>
              <p className="text-xs text-muted-foreground mb-1.5">Tone</p>
              <div className="flex rounded-md border overflow-hidden text-xs w-full">
                {(["formal", "conversational"] as CoverLetterTone[]).map((t) => (
                  <button
                    key={t}
                    onClick={() => setTone(t)}
                    className={`flex-1 py-1.5 capitalize transition-colors ${
                      tone === t
                        ? "bg-primary text-primary-foreground"
                        : "text-muted-foreground hover:text-foreground"
                    }`}
                  >
                    {t}
                  </button>
                ))}
              </div>
            </div>

            {/* Focus */}
            <div>
              <p className="text-xs text-muted-foreground mb-1.5">Emphasis</p>
              <div className="flex rounded-md border overflow-hidden text-xs w-full">
                {(["technical", "leadership", "culture_fit"] as CoverLetterFocus[]).map((f) => (
                  <button
                    key={f}
                    onClick={() => setFocus(f)}
                    className={`flex-1 py-1.5 transition-colors ${
                      focus === f
                        ? "bg-primary text-primary-foreground"
                        : "text-muted-foreground hover:text-foreground"
                    }`}
                  >
                    {f === "culture_fit" ? "Culture" : f.charAt(0).toUpperCase() + f.slice(1)}
                  </button>
                ))}
              </div>
            </div>

            {/* Generate */}
            <Button
              onClick={handleGenerate}
              disabled={!jdText.trim()}
              className="w-full"
              size="sm"
            >
              {status === "failed" ? "Retry Cover Letter" : "Generate Cover Letter"}
            </Button>

            {!jdText.trim() && (
              <p className="text-xs text-center text-muted-foreground">
                Paste a job description first to generate a cover letter.
              </p>
            )}
          </div>
        </div>
      </ScrollArea>
    </div>
  );
}
