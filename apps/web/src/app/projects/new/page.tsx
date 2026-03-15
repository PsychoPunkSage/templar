"use client";

// Template picker page — user selects a template, names their project, then
// clicks Create to be taken directly to the editor for that project.

import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useRouter } from "next/navigation";
import { useProjectStore } from "@/store/projectStore";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api } from "@/lib/api";
import { TemplateThumbnailPdf } from "@/components/pdf/TemplateThumbnailPdf";
import type { TemplateSummary } from "@templar/types";
import { Eye, X } from "lucide-react";

const MVP_USER_ID = "00000000-0000-0000-0000-000000000001";

function TemplatePickerCard({
  template,
  selected,
  onSelect,
  onPreview,
}: {
  template: TemplateSummary;
  selected: boolean;
  onSelect: () => void;
  onPreview: () => void;
}) {
  const thumbnailPdfUrl = api.getTemplateThumbnailPdfUrl(template.id);

  return (
    <button
      onClick={onSelect}
      className={`group text-left rounded-xl border-2 overflow-hidden transition-all hover:border-primary/60 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring ${
        selected
          ? "border-primary ring-2 ring-primary/20"
          : "border-border"
      }`}
    >
      {/* Thumbnail — renders thumbnail.pdf via PDF.js (compiled at Docker build time) */}
      <div className="aspect-[3/4] bg-muted relative overflow-hidden">
        <TemplateThumbnailPdf pdfUrl={thumbnailPdfUrl} />

        {/* Selection indicator */}
        {selected && (
          <div className="absolute top-2 right-2 w-5 h-5 rounded-full bg-primary flex items-center justify-center">
            <svg className="w-3 h-3 text-primary-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={3} d="M5 13l4 4L19 7" />
            </svg>
          </div>
        )}

        {/* Eye preview button — visible on hover (always visible on touch) */}
        <button
          onClick={(e) => { e.stopPropagation(); onPreview(); }}
          className="absolute top-2 left-2 z-10 flex h-7 w-7 items-center justify-center rounded-full bg-black/50 text-white opacity-0 transition-opacity group-hover:opacity-100 hover:bg-black/70 backdrop-blur-sm sm:opacity-0 max-sm:opacity-100"
          aria-label="Preview template"
        >
          <Eye className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* Template info */}
      <div className="p-3">
        <div className="font-medium text-sm">{template.name}</div>
        <p className="text-xs text-muted-foreground mt-0.5 line-clamp-2">
          {template.description}
        </p>
        <div className="flex flex-wrap gap-1 mt-2">
          {template.tags.map((tag) => (
            <span
              key={tag}
              className="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground"
            >
              {tag}
            </span>
          ))}
        </div>
      </div>
    </button>
  );
}

function TemplatePreviewModal({
  template,
  pdfUrl,
  onClose,
  onSelect,
}: {
  template: TemplateSummary;
  pdfUrl: string;
  onClose: () => void;
  onSelect: () => void;
}) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [onClose]);

  const modal = (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="relative m-auto flex overflow-hidden rounded-2xl bg-background shadow-2xl"
        style={{ width: "min(90vw, 1100px)", height: "90vh" }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Close button */}
        <button
          onClick={onClose}
          className="absolute top-3 right-3 z-10 flex h-7 w-7 items-center justify-center rounded-full bg-muted text-muted-foreground hover:bg-muted/80 transition-colors"
          aria-label="Close preview"
        >
          <X className="h-4 w-4" />
        </button>

        {/* Left: PDF preview */}
        <div className="flex-1 overflow-y-auto bg-muted p-6">
          <div className="w-full">
            <TemplateThumbnailPdf pdfUrl={pdfUrl} />
          </div>
        </div>

        {/* Right: metadata sidebar */}
        <div className="flex w-80 shrink-0 flex-col gap-4 border-l p-6">
          <div className="mt-2">
            <h2 className="text-lg font-semibold">{template.name}</h2>
            <p className="mt-2 text-sm text-muted-foreground">{template.description}</p>
          </div>

          <div className="flex flex-wrap gap-1">
            {template.tags.map((tag) => (
              <span
                key={tag}
                className="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground"
              >
                {tag}
              </span>
            ))}
          </div>

          <div className="mt-auto">
            <Button onClick={onSelect} className="w-full">
              Use this template
            </Button>
          </div>
        </div>
      </div>
    </div>
  );

  return createPortal(modal, document.body);
}

export default function NewProjectPage() {
  const router = useRouter();
  const { loadTemplates, templates, isLoadingTemplates } = useProjectStore();
  const [selectedTemplateId, setSelectedTemplateId] = useState<string | null>(null);
  const [previewTemplate, setPreviewTemplate] = useState<TemplateSummary | null>(null);
  const [projectName, setProjectName] = useState("");
  const [isCreating, setIsCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    loadTemplates();
  }, [loadTemplates]);

  // Auto-select the first template once loaded
  useEffect(() => {
    if (templates.length > 0 && !selectedTemplateId) {
      setSelectedTemplateId(templates[0].id);
    }
  }, [templates, selectedTemplateId]);

  const handleCreate = async () => {
    if (!selectedTemplateId || !projectName.trim()) return;

    setIsCreating(true);
    setError(null);

    try {
      const { createProject } = useProjectStore.getState();
      const project = await createProject(MVP_USER_ID, projectName.trim(), selectedTemplateId);
      // Redirect directly to the editor for this project
      router.push(`/editor/${project.id}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to create project");
      setIsCreating(false);
    }
  };

  return (
    <div className="max-w-3xl mx-auto px-6 py-10">
      <div className="mb-8">
        <h1 className="text-2xl font-bold tracking-tight">New Project</h1>
        <p className="text-sm text-muted-foreground mt-1">
          Choose a template and name your project.
        </p>
      </div>

      {/* Template picker grid */}
      <div className="mb-8">
        <h2 className="text-sm font-semibold mb-3">Choose a template</h2>

        {isLoadingTemplates ? (
          <div className="grid grid-cols-2 sm:grid-cols-3 gap-4">
            {[1, 2, 3].map((i) => (
              <div
                key={i}
                className="aspect-[3/4] rounded-xl bg-muted animate-pulse"
              />
            ))}
          </div>
        ) : templates.length === 0 ? (
          <div className="rounded-xl border border-dashed py-10 text-center text-sm text-muted-foreground">
            No templates available — the API may be starting up.
          </div>
        ) : (
          <div className="grid grid-cols-2 sm:grid-cols-3 gap-4">
            {templates.map((template) => (
              <TemplatePickerCard
                key={template.id}
                template={template}
                selected={selectedTemplateId === template.id}
                onSelect={() => setSelectedTemplateId(template.id)}
                onPreview={() => setPreviewTemplate(template)}
              />
            ))}
          </div>
        )}
      </div>

      {/* Project name + create button */}
      <div className="flex flex-col gap-3 max-w-md">
        <h2 className="text-sm font-semibold">Project name</h2>
        <Input
          placeholder="e.g. Senior SWE at Stripe"
          value={projectName}
          onChange={(e) => setProjectName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") handleCreate();
          }}
          disabled={isCreating}
        />

        {error && (
          <p className="text-xs text-destructive">{error}</p>
        )}

        <Button
          onClick={handleCreate}
          disabled={isCreating || !selectedTemplateId || !projectName.trim()}
        >
          {isCreating ? "Creating..." : "Create Project"}
        </Button>
      </div>

      {/* Full-screen template preview modal */}
      {previewTemplate && (
        <TemplatePreviewModal
          template={previewTemplate}
          pdfUrl={api.getTemplateThumbnailPdfUrl(previewTemplate.id)}
          onClose={() => setPreviewTemplate(null)}
          onSelect={() => {
            setSelectedTemplateId(previewTemplate.id);
            setPreviewTemplate(null);
          }}
        />
      )}
    </div>
  );
}
