"use client";

import { useEffect, useState } from "react";
import { usePersonaStore } from "@/store/personaStore";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent } from "@/components/ui/card";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import {
  Check,
  ChevronDown,
  ChevronUp,
  Lightbulb,
  Pencil,
  Plus,
  Trash2,
  User2,
  X,
} from "lucide-react";
import type { Persona, PersonaSuggestion } from "@templar/types";
import { useAuthStore } from "@/store/authStore";
import { MVP_USER_ID } from "@/store/resumeStore";
import {
  prefetchSuggestions,
  isDismissed,
  dismissSuggestion,
  dismissAll,
  invalidateCache,
} from "@/lib/personaSuggestCache";

const TONE_OPTIONS = [
  { value: "", label: "Auto-detect from JD" },
  { value: "startup", label: "Startup" },
  { value: "enterprise", label: "Enterprise" },
  { value: "research", label: "Research" },
  { value: "product", label: "Product" },
];

// ── Tag chip input ─────────────────────────────────────────────────────────

interface TagChipsProps {
  tags: string[];
  onAdd: (tag: string) => void;
  onRemove: (tag: string) => void;
  placeholder: string;
  chipClass: string;
}

function TagChips({ tags, onAdd, onRemove, placeholder, chipClass }: TagChipsProps) {
  const [input, setInput] = useState("");

  function commit() {
    const tag = input.trim().toLowerCase().replace(/,/g, "");
    if (tag && !tags.includes(tag)) onAdd(tag);
    setInput("");
  }

  return (
    <div className="flex flex-wrap gap-1.5 min-h-8 items-center p-2 rounded-md border border-input bg-background">
      {tags.map((tag) => (
        <span
          key={tag}
          className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium ${chipClass}`}
        >
          {tag}
          <button type="button" onClick={() => onRemove(tag)} className="hover:opacity-70">
            <X className="h-3 w-3" />
          </button>
        </span>
      ))}
      <input
        value={input}
        onChange={(e) => setInput(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === ",") {
            e.preventDefault();
            commit();
          }
        }}
        onBlur={commit}
        placeholder={tags.length === 0 ? placeholder : ""}
        className="flex-1 min-w-24 bg-transparent text-xs outline-none placeholder:text-muted-foreground"
      />
    </div>
  );
}

// ── Persona form ───────────────────────────────────────────────────────────

interface PersonaFormProps {
  initialName?: string;
  initialEmphasized?: string[];
  initialSuppressed?: string[];
  initialTone?: string;
  onSave: (name: string, emphasized: string[], suppressed: string[], tone: string) => Promise<void>;
  onCancel: () => void;
  isSaving: boolean;
}

function PersonaForm({
  initialName = "",
  initialEmphasized = [],
  initialSuppressed = [],
  initialTone = "",
  onSave,
  onCancel,
  isSaving,
}: PersonaFormProps) {
  const [name, setName] = useState(initialName);
  const [emphasized, setEmphasized] = useState<string[]>(initialEmphasized);
  const [suppressed, setSuppressed] = useState<string[]>(initialSuppressed);
  const [tone, setTone] = useState(initialTone);

  async function handleSave() {
    if (!name.trim()) return;
    await onSave(name.trim(), emphasized, suppressed, tone);
  }

  return (
    <div className="flex flex-col gap-3">
      <div>
        <label className="text-xs font-medium text-muted-foreground mb-1 block">Name</label>
        <Input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="e.g. ML Engineer, PM Track"
          className="h-8 text-sm"
          onKeyDown={(e) => e.key === "Enter" && handleSave()}
        />
      </div>

      <div>
        <label className="text-xs font-medium text-muted-foreground mb-1 block">
          Boost tags <span className="font-normal">(press Enter or comma to add)</span>
        </label>
        <TagChips
          tags={emphasized}
          onAdd={(t) => setEmphasized((prev) => [...prev, t])}
          onRemove={(t) => setEmphasized((prev) => prev.filter((x) => x !== t))}
          placeholder="ml, python, tensorflow…"
          chipClass="bg-indigo-100 dark:bg-indigo-900/40 text-indigo-700 dark:text-indigo-300"
        />
      </div>

      <div>
        <label className="text-xs font-medium text-muted-foreground mb-1 block">
          Suppress tags
        </label>
        <TagChips
          tags={suppressed}
          onAdd={(t) => setSuppressed((prev) => [...prev, t])}
          onRemove={(t) => setSuppressed((prev) => prev.filter((x) => x !== t))}
          placeholder="react, frontend…"
          chipClass="bg-muted text-muted-foreground"
        />
      </div>

      <div>
        <label className="text-xs font-medium text-muted-foreground mb-1 block">
          Tone preference
        </label>
        <select
          value={tone}
          onChange={(e) => setTone(e.target.value)}
          className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
        >
          {TONE_OPTIONS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </div>

      <div className="flex items-center gap-2 pt-1">
        <Button size="sm" onClick={handleSave} disabled={!name.trim() || isSaving}>
          {isSaving ? "Saving…" : "Save"}
        </Button>
        <Button size="sm" variant="ghost" onClick={onCancel} disabled={isSaving}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

// ── Persona card ───────────────────────────────────────────────────────────

interface PersonaCardProps {
  persona: Persona;
  onEdit: (
    id: string,
    name: string,
    emphasized: string[],
    suppressed: string[],
    tone: string,
  ) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
  isSaving: boolean;
}

function PersonaCard({ persona, onEdit, onDelete, isSaving }: PersonaCardProps) {
  const [isEditing, setIsEditing] = useState(false);

  async function handleSave(
    name: string,
    emphasized: string[],
    suppressed: string[],
    tone: string,
  ) {
    await onEdit(persona.id, name, emphasized, suppressed, tone);
    setIsEditing(false);
  }

  const toneLabel = TONE_OPTIONS.find((o) => o.value === (persona.tone_preference ?? ""))?.label;

  return (
    <Card className="border border-border">
      <CardContent className="p-4 flex flex-col gap-3">
        {isEditing ? (
          <PersonaForm
            initialName={persona.name}
            initialEmphasized={persona.emphasized_tags}
            initialSuppressed={persona.suppressed_tags}
            initialTone={persona.tone_preference ?? ""}
            onSave={handleSave}
            onCancel={() => setIsEditing(false)}
            isSaving={isSaving}
          />
        ) : (
          <>
            <div className="flex items-center justify-between gap-2">
              <span className="font-medium text-sm truncate">{persona.name}</span>
              <div className="flex items-center gap-1 shrink-0">
                <button
                  onClick={() => setIsEditing(true)}
                  className="p-1 rounded hover:bg-accent text-muted-foreground hover:text-foreground transition-colors"
                  title="Edit"
                >
                  <Pencil className="h-3.5 w-3.5" />
                </button>
                <AlertDialog>
                  <AlertDialogTrigger asChild>
                    <button
                      className="p-1 rounded hover:bg-destructive/10 text-muted-foreground hover:text-destructive transition-colors"
                      title="Delete"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                  </AlertDialogTrigger>
                  <AlertDialogContent>
                    <AlertDialogHeader>
                      <AlertDialogTitle>Delete persona?</AlertDialogTitle>
                      <AlertDialogDescription>
                        &ldquo;{persona.name}&rdquo; will be permanently removed.
                      </AlertDialogDescription>
                    </AlertDialogHeader>
                    <AlertDialogFooter>
                      <AlertDialogCancel>Cancel</AlertDialogCancel>
                      <AlertDialogAction
                        onClick={() => onDelete(persona.id)}
                        className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                      >
                        Delete
                      </AlertDialogAction>
                    </AlertDialogFooter>
                  </AlertDialogContent>
                </AlertDialog>
              </div>
            </div>

            {persona.emphasized_tags.length > 0 && (
              <div className="flex flex-wrap gap-1">
                {persona.emphasized_tags.map((t) => (
                  <span
                    key={t}
                    className="rounded-full bg-indigo-100 dark:bg-indigo-900/40 px-2 py-0.5 text-xs font-medium text-indigo-700 dark:text-indigo-300"
                  >
                    {t}
                  </span>
                ))}
              </div>
            )}
            {persona.suppressed_tags.length > 0 && (
              <div className="flex flex-wrap gap-1">
                {persona.suppressed_tags.map((t) => (
                  <span
                    key={t}
                    className="rounded-full bg-muted px-2 py-0.5 text-xs font-medium text-muted-foreground line-through"
                  >
                    {t}
                  </span>
                ))}
              </div>
            )}

            {persona.tone_preference && (
              <span className="self-start rounded-md border border-border px-2 py-0.5 text-xs text-muted-foreground">
                {toneLabel}
              </span>
            )}

            {persona.emphasized_tags.length === 0 && persona.suppressed_tags.length === 0 && (
              <p className="text-xs text-muted-foreground">
                No tags set — edit to add boosts or suppressions.
              </p>
            )}
          </>
        )}
      </CardContent>
    </Card>
  );
}

// ── Suggestion mini-card ───────────────────────────────────────────────────

interface SuggestionMiniCardProps {
  suggestion: PersonaSuggestion;
  isAdded: boolean;
  onAdd: (s: PersonaSuggestion) => void;
  onDismiss: (name: string) => void;
}

function SuggestionMiniCard({ suggestion, isAdded, onAdd, onDismiss }: SuggestionMiniCardProps) {
  return (
    <div className={`relative rounded-lg border p-3 flex flex-col gap-2 bg-background transition-opacity ${isAdded ? "opacity-40" : ""}`}>
      {isAdded && (
        <div className="absolute inset-0 flex items-center justify-center rounded-lg bg-background/60 backdrop-blur-[1px] z-10 pointer-events-none">
          <span className="flex items-center gap-1 text-xs font-medium text-green-600 dark:text-green-400">
            <Check className="h-3.5 w-3.5" />
            Added
          </span>
        </div>
      )}

      <div className="flex items-center justify-between gap-2">
        <span className="text-sm font-medium truncate">{suggestion.name}</span>
        <div className="flex items-center gap-1 shrink-0">
          <Button
            size="sm"
            variant="outline"
            className="h-7 text-xs"
            onClick={() => onAdd(suggestion)}
            disabled={isAdded}
          >
            {isAdded ? <Check className="h-3 w-3 mr-1" /> : null}
            {isAdded ? "Added" : "Add"}
          </Button>
          <button
            onClick={() => onDismiss(suggestion.name)}
            className="p-1 rounded hover:bg-accent text-muted-foreground hover:text-foreground transition-colors"
            title="Dismiss"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      {suggestion.emphasized_tags.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {suggestion.emphasized_tags.map((t) => (
            <span
              key={t}
              className="rounded-full bg-indigo-100 dark:bg-indigo-900/40 px-2 py-0.5 text-xs font-medium text-indigo-700 dark:text-indigo-300"
            >
              {t}
            </span>
          ))}
        </div>
      )}
      <p className="text-xs text-muted-foreground leading-relaxed line-clamp-2">
        {suggestion.reasoning}
      </p>
    </div>
  );
}

// ── Page ───────────────────────────────────────────────────────────────────

export default function PersonasPage() {
  const {
    personas,
    isSaving,
    error,
    loadPersonas,
    createPersona,
    updatePersona,
    deletePersona,
    clearError,
  } = usePersonaStore();
  const userId = useAuthStore((s) => s.internalUserId) ?? MVP_USER_ID;
  const isAuthReady = useAuthStore((s) => s.isAuthReady);

  // pageReady gates the skeleton: false until BOTH loadPersonas AND prefetchSuggestions
  // have settled for the first time. Initialized to true when navigating back to this
  // page with the store already warm (stale-while-revalidate — no skeleton on tab switch).
  const [pageReady, setPageReady] = useState(() => personas.length > 0);
  const [showCreate, setShowCreate] = useState(false);
  const [suggestions, setSuggestions] = useState<PersonaSuggestion[]>([]);
  const [suggestionsOpen, setSuggestionsOpen] = useState(true);
  const [prefillData, setPrefillData] = useState<PersonaSuggestion | null>(null);
  // Tracks which suggestion names the user has accepted (optimistic visual state)
  const [addedNames, setAddedNames] = useState<Set<string>>(new Set());
  // Forces re-render when a per-card dismiss fires (dismissed Set is module-level, not React state)
  const [dismissRevision, setDismissRevision] = useState(0);

  // On mount: wait for BOTH persona list and suggestions to settle before showing content.
  // This prevents the half-loaded state where personas appear without suggestions (or vice versa).
  // On tab-switch the store is warm → pageReady starts true → no skeleton, grid shows immediately.
  useEffect(() => {
    if (!isAuthReady) return;
    Promise.all([
      loadPersonas(),
      prefetchSuggestions(userId, setSuggestions),
    ]).finally(() => setPageReady(true));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isAuthReady]);

  // Pre-populate addedNames when either list resolves (handles the load-order race).
  useEffect(() => {
    if (suggestions.length === 0 || personas.length === 0) return;
    const existingNames = new Set(personas.map((p) => p.name.toLowerCase()));
    setAddedNames(
      new Set(
        suggestions
          .filter((s) => existingNames.has(s.name.toLowerCase()))
          .map((s) => s.name),
      ),
    );
  }, [suggestions, personas]);

  // visibleSuggestions excludes accepted + dismissed entries
  const visibleSuggestions = suggestions.filter(
    (s) => !addedNames.has(s.name) && !isDismissed(s.name),
  );

  function handleAddSuggestion(s: PersonaSuggestion) {
    setAddedNames((prev) => new Set([...prev, s.name])); // optimistic
    setPrefillData(s);
    setShowCreate(true);
  }

  function handleDismissSuggestion(name: string) {
    dismissSuggestion(name);
    setDismissRevision((v) => v + 1); // force re-render
  }

  function handleDismissAll() {
    dismissAll(visibleSuggestions.map((s) => s.name));
    setDismissRevision((v) => v + 1);
  }

  async function handleCreate(
    name: string,
    emphasized: string[],
    suppressed: string[],
    tone: string,
  ) {
    await createPersona(name, emphasized, suppressed, tone || undefined);
    setShowCreate(false);
    setPrefillData(null);
    invalidateCache(); // next page visit will re-check hashes
  }

  async function handleEdit(
    id: string,
    name: string,
    emphasized: string[],
    suppressed: string[],
    tone: string,
  ) {
    await updatePersona(id, {
      name,
      emphasized_tags: emphasized,
      suppressed_tags: suppressed,
      tone_preference: tone || null,
    });
  }

  return (
    <div className="max-w-4xl mx-auto px-6 py-8 flex flex-col gap-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold">Personas</h1>
          <p className="text-sm text-muted-foreground mt-0.5">
            Bias content selection for different career tracks without changing your context.
          </p>
        </div>
        {!showCreate && (
          <Button size="sm" onClick={() => setShowCreate(true)}>
            <Plus className="h-4 w-4 mr-1.5" />
            New persona
          </Button>
        )}
      </div>

      {/* Error banner */}
      {error && (
        <div className="px-3 py-2 bg-destructive/10 text-destructive text-sm rounded-md flex items-center justify-between">
          <span>{error}</span>
          <button onClick={clearError}>
            <X className="h-4 w-4" />
          </button>
        </div>
      )}

      {/* Inline create form */}
      {showCreate && (
        <Card className="border border-indigo-200 dark:border-indigo-800">
          <CardContent className="p-4">
            <p className="text-sm font-medium mb-3">
              {prefillData ? `Add persona: ${prefillData.name}` : "New persona"}
            </p>
            <PersonaForm
              key={prefillData ? `prefill-${prefillData.name}` : "blank"}
              initialName={prefillData?.name ?? ""}
              initialEmphasized={prefillData?.emphasized_tags ?? []}
              initialSuppressed={prefillData?.suppressed_tags ?? []}
              initialTone={prefillData?.tone_preference ?? ""}
              onSave={handleCreate}
              onCancel={() => {
                setShowCreate(false);
                setPrefillData(null);
              }}
              isSaving={isSaving}
            />
          </CardContent>
        </Card>
      )}

      {/* Suggestion strip — shown when the user already has personas + there are visible suggestions */}
      {personas.length > 0 && visibleSuggestions.length > 0 && (
        <div className="rounded-lg border bg-yellow-50 dark:bg-yellow-950/20 border-yellow-200 dark:border-yellow-800">
          <div className="flex items-center justify-between px-4 py-2.5">
            <button
              className="flex items-center gap-2 text-sm font-medium text-yellow-800 dark:text-yellow-200"
              onClick={() => setSuggestionsOpen((v) => !v)}
            >
              <Lightbulb className="h-4 w-4" />
              Suggested personas based on your context
              {suggestionsOpen ? (
                <ChevronUp className="h-4 w-4" />
              ) : (
                <ChevronDown className="h-4 w-4" />
              )}
            </button>
            <button
              onClick={handleDismissAll}
              className="text-muted-foreground hover:text-foreground"
              title="Dismiss all"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
          {suggestionsOpen && (
            <div className="px-4 pb-4 grid grid-cols-1 md:grid-cols-3 gap-3">
              {visibleSuggestions.map((s) => (
                <SuggestionMiniCard
                  key={s.name}
                  suggestion={s}
                  isAdded={addedNames.has(s.name)}
                  onAdd={handleAddSuggestion}
                  onDismiss={handleDismissSuggestion}
                />
              ))}
            </div>
          )}
        </div>
      )}

      {/* ── Persona list / empty state ────────────────────────────────── */}
      {!pageReady ? (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {[1, 2, 3].map((i) => (
            <div key={i} className="h-32 rounded-lg bg-muted animate-pulse" />
          ))}
        </div>
      ) : personas.length === 0 && !showCreate ? (
        <>
          {/*
           * Section A — always visible, zero latency.
           * No dependency on suggestions. User can create manually right away.
           */}
          <div className="flex flex-col items-center justify-center py-16 text-center gap-3">
            <div className="rounded-full bg-muted p-4">
              <User2 className="h-6 w-6 text-muted-foreground" />
            </div>
            <p className="text-sm font-medium">No personas yet</p>
            <p className="text-xs text-muted-foreground max-w-xs">
              Create a persona to bias resume content selection for ML Engineer, PM, or any other
              track.
            </p>
            <Button size="sm" variant="outline" onClick={() => setShowCreate(true)}>
              <Plus className="h-4 w-4 mr-1.5" />
              Create manually
            </Button>
          </div>

          {/*
           * Section B — appears only when the background prefetch resolves with suggestions.
           * No skeleton, no loading spinner. Appears silently when data is ready.
           * Fully independent of Section A — neither blocks the other.
           */}
          {suggestions.length > 0 && visibleSuggestions.length > 0 && (
            <div className="flex flex-col gap-4">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2 text-sm text-muted-foreground">
                  <Lightbulb className="h-4 w-4" />
                  <span>
                    Based on your context — review and add to get started
                  </span>
                </div>
                <button
                  onClick={handleDismissAll}
                  className="text-muted-foreground hover:text-foreground text-xs flex items-center gap-1"
                  title="Dismiss all suggestions"
                >
                  <X className="h-3.5 w-3.5" />
                  Dismiss
                </button>
              </div>
              <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                {visibleSuggestions.map((s) => (
                  <SuggestionMiniCard
                    key={s.name}
                    suggestion={s}
                    isAdded={addedNames.has(s.name)}
                    onAdd={handleAddSuggestion}
                    onDismiss={handleDismissSuggestion}
                  />
                ))}
              </div>
            </div>
          )}
        </>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {personas.map((p) => (
            <PersonaCard
              key={p.id}
              persona={p}
              onEdit={handleEdit}
              onDelete={deletePersona}
              isSaving={isSaving}
            />
          ))}
        </div>
      )}
    </div>
  );
}
