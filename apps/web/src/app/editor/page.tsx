"use client";

import { JdInput } from "@/components/editor/JdInput";
import { BulletList } from "@/components/editor/BulletList";
import { FitReportPanel } from "@/components/editor/FitReportPanel";
import { ContextPanel } from "@/components/editor/ContextPanel";
import { PdfPreview } from "@/components/pdf/PdfPreview";
import { ThemeToggle } from "@/components/ThemeToggle";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useResumeStore } from "@/store/resumeStore";

/**
 * Split-pane resume editor page.
 *
 * Left pane: tabbed — "Generate" (JD input + fit report + bullets) and
 *             "Context" (context ingestion panel)
 * Right pane: PDF.js live preview (debounced at 300ms minimum)
 *
 * This is a client component because it uses Zustand hooks and
 * interactive event handlers throughout.
 */
export default function EditorPage() {
  const { generate, isGenerating, error, clearError, bullets } =
    useResumeStore();

  return (
    <div className="flex flex-col h-screen bg-background">
      {/* Header */}
      <header className="flex items-center justify-between px-6 py-3 border-b bg-background shrink-0">
        <div className="flex items-center gap-3">
          <span className="font-bold text-lg tracking-tight">Templar</span>
          <span className="text-xs text-muted-foreground hidden sm:block">
            AI Resume Engine
          </span>
        </div>
        <div className="flex items-center gap-2">
          {bullets.length > 0 && (
            <span className="text-xs text-muted-foreground">
              {bullets.length} bullets
            </span>
          )}
          <ThemeToggle />
          <Button
            onClick={() => generate()}
            disabled={isGenerating}
            size="sm"
          >
            {isGenerating ? "Generating..." : "Generate Resume"}
          </Button>
        </div>
      </header>

      {/* Error banner */}
      {error && (
        <div className="mx-6 mt-2 px-3 py-2 bg-destructive/10 text-destructive text-sm rounded-md flex items-center justify-between shrink-0">
          <span>{error}</span>
          <button
            onClick={clearError}
            className="font-bold ml-4 hover:opacity-70 transition-opacity"
            aria-label="Dismiss error"
          >
            x
          </button>
        </div>
      )}

      {/* Split pane */}
      <div className="flex flex-1 overflow-hidden">
        {/* Left: Editor panel */}
        <div className="w-1/2 border-r flex flex-col overflow-hidden">
          <ScrollArea className="flex-1">
            <div className="p-4">
              <Tabs defaultValue="generate">
                <TabsList className="w-full mb-4">
                  <TabsTrigger value="generate" className="flex-1">
                    Generate
                  </TabsTrigger>
                  <TabsTrigger value="context" className="flex-1">
                    Context
                  </TabsTrigger>
                </TabsList>
                <TabsContent value="generate" className="flex flex-col gap-4">
                  <JdInput />
                  <FitReportPanel />
                  <BulletList />
                </TabsContent>
                <TabsContent value="context">
                  <ContextPanel />
                </TabsContent>
              </Tabs>
            </div>
          </ScrollArea>
        </div>

        {/* Right: PDF preview panel */}
        <div className="w-1/2 bg-muted/30 relative overflow-hidden">
          <PdfPreview />
        </div>
      </div>
    </div>
  );
}
