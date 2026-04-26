import type {
  GenerateJobResponse,
  GenerationStatusResponse,
  ResumeDetailResponse,
  AuditManifest,
  TemplateListResponse,
  CvProject,
  ProjectListResponse,
  CreateProjectRequest,
  UpdateProjectRequest,
  ContextEntriesResponse,
  FitScoreResponse,
  FitReport,
  UserProfileResponse,
  UpsertProfileRequest,
  ProfileLinkData,
  Persona,
  PersonaListResponse,
  CreatePersonaRequest,
  UpdatePersonaRequest,
  PersonaSuggestion,
  SuggestPersonasResponse,
  PrepResponse,
  PrepStatusResponse,
  PrepMeta,
  UpdateCompanyRequest,
} from "@templar/types";

export interface CoverLetterResponse {
  id: string;
  user_id: string;
  resume_id: string | null;
  persona_id: string | null;
  jd_text_hash: string;
  tone: string;
  focus: string;
  content: Array<{ role: string; text: string }>;
  company_name: string | null;
  role_title: string | null;
  created_at: string;
}

/**
 * Response from POST /api/v1/resumes/fit-score/cached.
 * cache_hit is always true — the endpoint only returns data on a cache hit.
 */
interface CachedFitScoreResponse {
  fit_report: FitReport;
  cache_hit: boolean;
  jd_hash: string;
  context_hash: string;
}

export type {
  ContextEntryRow,
  CompletenessReport,
  CompletenessSection,
  ContextEntriesResponse,
} from "@templar/types";

const API_BASE =
  process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";

// ── Auth token injection ───────────────────────────────────────────────────
// AuthSync calls setTokenGetter() once on mount with Clerk's getToken function.
// Every apiFetch call then asks for a fresh token — Clerk caches internally and
// only refreshes when the token is near expiry, so this is cheap.

let _tokenGetter: (() => Promise<string | null>) | null = null;

export function setTokenGetter(fn: () => Promise<string | null>): void {
  _tokenGetter = fn;
}

async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const token = _tokenGetter ? await _tokenGetter() : null;
  const authHeader: Record<string, string> = token
    ? { Authorization: `Bearer ${token}` }
    : {};

  const res = await fetch(`${API_BASE}${path}`, {
    headers: { "Content-Type": "application/json", ...authHeader },
    ...init,
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({})) as Record<string, unknown>;
    const errorMsg =
      (err?.error as Record<string, unknown>)?.message ??
      `HTTP ${res.status}`;
    throw new Error(String(errorMsg));
  }
  if (res.status === 204 || res.headers.get('content-length') === '0') {
    return undefined as T;
  }
  return res.json() as Promise<T>;
}

export const api = {
  /**
   * POST /api/v1/resumes/fit-score
   * Analyses fit between user context and a job description.
   * Returns a FitReport with cache metadata (cache_hit, jd_hash, context_hash).
   * Pass forceRefresh=true to bypass the cache and always re-score via LLM.
   */
  analyzeFit: (userId: string, jdText: string, forceRefresh = false) =>
    apiFetch<FitScoreResponse>("/api/v1/resumes/fit-score", {
      method: "POST",
      body: JSON.stringify({ user_id: userId, jd_text: jdText, force_refresh: forceRefresh }),
    }),

  /**
   * POST /api/v1/resumes/fit-score/cached
   * Cache-only fit score lookup — never calls the LLM.
   * Returns the cached FitReport on a hit, or null on a miss (404) or any network error.
   * Callers should treat null as a silent no-op (no error surfaced to the user).
   */
  getCachedFitScore: async (userId: string, jdText: string): Promise<CachedFitScoreResponse | null> => {
    try {
      return await apiFetch<CachedFitScoreResponse>("/api/v1/resumes/fit-score/cached", {
        method: "POST",
        body: JSON.stringify({ user_id: userId, jd_text: jdText }),
      });
    } catch {
      // 404 = cache miss; any other error = silently treated as miss
      return null;
    }
  },

  /**
   * POST /api/v1/resumes/generate  (FIX-08)
   * Enqueues an async generation job and returns immediately with { job_id, status: "queued" }.
   * The actual pipeline runs in the background worker.
   * Poll GET /api/v1/generation/jobs/:id/status to track progress.
   *
   * @param resumeMode 'single_page' (default) or 'cv' — controls pagination in the pipeline.
   */
  generateResume: (
    userId: string,
    jdText: string,
    resumeMode: 'single_page' | 'cv' = 'single_page',
    personaId?: string | null,
  ) =>
    apiFetch<GenerateJobResponse>("/api/v1/resumes/generate", {
      method: "POST",
      body: JSON.stringify({
        user_id: userId,
        jd_text: jdText,
        resume_mode: resumeMode,
        ...(personaId ? { persona_id: personaId } : {}),
      }),
    }),

  /**
   * GET /api/v1/generation/jobs/:id/status  (FIX-08 + FIX-10)
   * Polls the status of an async generation job.
   * On status='done': entry_groups, fit_report, and layout_flagged are populated.
   * On status='failed': error is populated.
   */
  getGenerationStatus: (jobId: string) =>
    apiFetch<GenerationStatusResponse>(`/api/v1/generation/jobs/${jobId}/status`),

  /**
   * GET /api/v1/resumes/:id
   * Fetches a resume and its bullets from the database.
   */
  getResume: (id: string) =>
    apiFetch<ResumeDetailResponse>(`/api/v1/resumes/${id}`),

  /**
   * GET /api/v1/resumes/:id/audit
   * Fetches the grounding audit manifest for a resume.
   */
  getAuditManifest: (id: string) =>
    apiFetch<AuditManifest>(`/api/v1/resumes/${id}/audit`),

  /**
   * GET /api/v1/resumes/:id/render-job
   * Returns the latest render job for a resume (job_id + status).
   * Used on page load to restore render state after a refresh.
   * Returns null if no render job exists yet (404).
   */
  getResumeRenderJob: async (resumeId: string): Promise<{ job_id: string; status: string } | null> => {
    const res = await fetch(`${API_BASE}/api/v1/resumes/${resumeId}/render-job`);
    if (!res.ok) return null;
    return res.json() as Promise<{ job_id: string; status: string }>;
  },

  /**
   * POST /api/v1/render
   * Triggers a Tectonic LaTeX → PDF render job.
   */
  triggerRender: (resumeId: string) =>
    apiFetch<{ job_id: string }>("/api/v1/render", {
      method: "POST",
      body: JSON.stringify({ resume_id: resumeId }),
    }),

  /**
   * GET /api/v1/render/:job_id/status
   * Polls the status of a render job.
   * Returns status ('queued' | 'processing' | 'done' | 'failed') and
   * error_message (only set when status='failed').
   */
  getRenderStatus: (jobId: string) =>
    apiFetch<{ status: string; error_message?: string | null }>(
      `/api/v1/render/${jobId}/status`
    ),

  /**
   * Returns the URL to stream the rendered PDF.
   * GET /api/v1/render/:job_id
   */
  getPdfUrl: (jobId: string) => `${API_BASE}/api/v1/render/${jobId}`,

  /**
   * Downloads the rendered PDF for a job via a Blob URL.
   * Uses fetch() + createObjectURL so the browser download dialog
   * works cross-origin (a.download is silently ignored for cross-origin URLs).
   * Revokes the blob URL immediately after the click — safe because the browser
   * keeps the Blob alive until the download starts.
   */
  downloadPdf: async (jobId: string, filename?: string): Promise<void> => {
    const res = await fetch(`${API_BASE}/api/v1/render/${jobId}`, {
      headers: { Accept: "application/pdf" },
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({})) as Record<string, unknown>;
      const msg = (err?.error as Record<string, unknown>)?.message ?? `HTTP ${res.status}`;
      throw new Error(String(msg));
    }
    const blob = await res.blob();
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename ?? `templar-resume-${new Date().toISOString().split("T")[0]}.pdf`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  },

  // ── Templates API ──────────────────────────────────────────────────────────

  /**
   * GET /api/v1/templates
   * Lists all available resume templates.
   * Response is cached (Cache-Control: public, max-age=300) so repeated calls are cheap.
   */
  listTemplates: () =>
    apiFetch<TemplateListResponse>("/api/v1/templates"),

  /**
   * Returns the full URL for a template thumbnail image.
   * The thumbnail is served via the API (proxied from S3) so MinIO internal
   * hostnames don't leak to the browser in dev environments.
   */
  getTemplateThumbnailUrl: (thumbnailPath: string) =>
    `${API_BASE}${thumbnailPath}`,

  /**
   * Returns the URL for the on-demand template PDF render endpoint.
   * GET /api/v1/templates/:id/render-pdf
   * Response is a raw PDF with Content-Type: application/pdf.
   * The backend caches the compiled result in-process after the first request.
   */
  getTemplateRenderPdfUrl: (id: string): string =>
    `${API_BASE}/api/v1/templates/${id}/render-pdf`,

  /**
   * Returns the URL for the template thumbnail PDF endpoint.
   * GET /api/v1/templates/:id/thumbnail-pdf
   * Serves thumbnail.pdf (compact card-sized PDF) — used in the template picker grid.
   * Falls back to template.tex compile if thumbnail.tex is missing.
   */
  getTemplateThumbnailPdfUrl: (id: string): string =>
    `${API_BASE}/api/v1/templates/${id}/thumbnail-pdf`,

  // ── Projects API ───────────────────────────────────────────────────────────

  /**
   * GET /api/v1/projects?user_id={uuid}
   * Lists all projects for a user, ordered by updated_at DESC.
   */
  listProjects: (userId: string) =>
    apiFetch<ProjectListResponse>(`/api/v1/projects?user_id=${userId}`),

  /**
   * POST /api/v1/projects
   * Creates a new project. Returns 400 if template_id is unknown.
   */
  createProject: (body: CreateProjectRequest) =>
    apiFetch<CvProject>("/api/v1/projects", {
      method: "POST",
      body: JSON.stringify(body),
    }),

  /**
   * GET /api/v1/projects/:id
   */
  getProject: (id: string) =>
    apiFetch<CvProject>(`/api/v1/projects/${id}`),

  /**
   * PATCH /api/v1/projects/:id
   * Partial update — only provided fields are changed.
   */
  updateProject: (id: string, body: UpdateProjectRequest) =>
    apiFetch<CvProject>(`/api/v1/projects/${id}`, {
      method: "PATCH",
      body: JSON.stringify(body),
    }),

  /**
   * DELETE /api/v1/projects/:id
   * Hard-deletes the project. Resume rows are NOT deleted (FK is SET NULL).
   */
  deleteProject: (id: string) =>
    fetch(`${API_BASE}/api/v1/projects/${id}`, { method: "DELETE" }),

  // ── Context Library API ────────────────────────────────────────────────────

  /**
   * GET /api/v1/context?user_id={userId}
   * Returns all current context entries (latest version per entry_id) and a
   * completeness health report.
   */
  getContextEntries: (userId: string) =>
    apiFetch<ContextEntriesResponse>(`/api/v1/context?user_id=${userId}`),

  /**
   * PATCH /api/v1/context/entries/:entryId/evergreen
   * Flips the flagged_evergreen flag on an entry (append-only — creates new version).
   * Only meaningful for skill / certification entries, but the API accepts any type.
   */
  toggleEvergreen: (entryId: string, userId: string, value: boolean) =>
    apiFetch<void>(`/api/v1/context/entries/${entryId}/evergreen`, {
      method: "PATCH",
      body: JSON.stringify({ flagged_evergreen: value, user_id: userId }),
    }),

  /**
   * PATCH /api/v1/context/entries/:entryId
   * Partial update of whitelisted data fields (append-only — creates new version).
   * Only fields in the server-side EDITABLE_FIELDS whitelist are applied.
   */
  patchEntry: (entryId: string, userId: string, patch: Record<string, unknown>) =>
    apiFetch<void>(`/api/v1/context/entries/${entryId}`, {
      method: "PATCH",
      body: JSON.stringify({ user_id: userId, patch }),
    }),

  /**
   * DELETE /api/v1/context/entries/:entryId?user_id={uuid}
   * Hard-deletes all versions of a context entry for the given user.
   * Returns 204 No Content on success.
   */
  deleteContextEntry: async (entryId: string, userId: string): Promise<void> => {
    const res = await fetch(
      `${API_BASE}/api/v1/context/entries/${entryId}?user_id=${userId}`,
      { method: "DELETE" }
    );
    if (!res.ok && res.status !== 204) throw new Error(`HTTP ${res.status}`);
  },

  /**
   * DELETE /api/v1/context?user_id={uuid}
   * Hard-deletes ALL context entries for the given user.
   * Returns the count of deleted entries.
   */
  clearAllContext: async (userId: string): Promise<{ deleted_count: number }> =>
    apiFetch<{ deleted_count: number }>(`/api/v1/context?user_id=${userId}`, {
      method: "DELETE",
    }),

  // ── Inline bullet refinement ───────────────────────────────────────────────

  /**
   * POST /api/v1/resumes/:resumeId/bullets/refine
   * Refines a single bullet in-place using the user's instruction.
   * Context is scoped to the bullet's source entry — not the full user context.
   * On was_rejected=true: caller should revert to original_text (grounding failed).
   */
  refineBullet: (
    resumeId: string,
    payload: {
      bullet_text: string;
      source_entry_id: string;
      section: string;
      instruction: string;
    }
  ) =>
    apiFetch<{
      original_text: string;
      refined_text: string;
      verified_line_count: number;
      grounding_score: number;
      verdict: string;
      was_rejected: boolean;
      rejection_reason: string | null;
    }>(`/api/v1/resumes/${resumeId}/bullets/refine`, {
      method: "POST",
      body: JSON.stringify(payload),
    }),

  // ── Personas API ───────────────────────────────────────────────────────────

  listPersonas: (userId: string) =>
    apiFetch<PersonaListResponse>(`/api/v1/personas?user_id=${userId}`),

  createPersona: (body: CreatePersonaRequest) =>
    apiFetch<Persona>("/api/v1/personas", {
      method: "POST",
      body: JSON.stringify(body),
    }),

  updatePersona: (id: string, body: UpdatePersonaRequest) =>
    apiFetch<Persona>(`/api/v1/personas/${id}`, {
      method: "PATCH",
      body: JSON.stringify(body),
    }),

  deletePersona: (id: string) =>
    fetch(`${API_BASE}/api/v1/personas/${id}`, { method: "DELETE" }),

  /**
   * GET /api/v1/personas/suggest
   * Returns 200 SuggestPersonasResponse on fresh data (or first call).
   * Returns undefined (204 No Content) when the client's hashes match the server's —
   * the cached result is still valid and no LLM call was made.
   */
  suggestPersonas: (userId: string, ctxHash?: string, personaHash?: string) =>
    apiFetch<SuggestPersonasResponse>(
      `/api/v1/personas/suggest?user_id=${userId}` +
        (ctxHash ? `&ctx_hash=${encodeURIComponent(ctxHash)}` : "") +
        (personaHash ? `&persona_hash=${encodeURIComponent(personaHash)}` : ""),
    ) as Promise<SuggestPersonasResponse | undefined>,

  getProfile: (userId: string) =>
    apiFetch<UserProfileResponse>(`/api/v1/profile?user_id=${userId}`),

  updateProfile: (req: UpsertProfileRequest) =>
    apiFetch<UserProfileResponse>("/api/v1/profile", {
      method: "PUT",
      body: JSON.stringify(req),
    }),

  /**
   * GET /api/v1/auth/me
   * Resolves the caller's internal UUID from the Clerk JWT.
   * On success returns { user_id: string } (UUID).
   * If CLERK_JWKS_URL is not configured on the server, returns the seed MVP UUID
   * so dev/test environments work without Clerk credentials.
   */
  authMe: () =>
    apiFetch<{ user_id: string }>("/api/v1/auth/me"),

  // ── Cover Letter API (Phase 3) ─────────────────────────────────────────────

  generateCoverLetter: (
    userId: string,
    jdText: string,
    tone: string,
    focus: string,
    resumeId: string | null,
    personaId: string | null,
  ) =>
    apiFetch<CoverLetterResponse>("/api/v1/cover-letters/generate", {
      method: "POST",
      body: JSON.stringify({
        user_id: userId,
        jd_text: jdText,
        tone,
        focus,
        resume_id: resumeId ?? undefined,
        persona_id: personaId ?? undefined,
      }),
    }),

  getCoverLetter: (id: string) =>
    apiFetch<CoverLetterResponse>(`/api/v1/cover-letters/${id}`),

  listCoverLetters: (userId: string, resumeId?: string) =>
    apiFetch<{ cover_letters: CoverLetterResponse[] }>(
      `/api/v1/cover-letters?user_id=${userId}` +
        (resumeId ? `&resume_id=${resumeId}` : ""),
    ),

  // ── Interview Prep API (Phase 3 Power) ────────────────────────────────────

  /**
   * GET /api/v1/interview-prep/:project_id
   * Returns full prep package (meta + bullets). Returns {meta: null, bullets: []}
   * if no prep has been generated yet.
   */
  getInterviewPrep: (projectId: string) =>
    apiFetch<PrepResponse>(`/api/v1/interview-prep/${projectId}`),

  /**
   * POST /api/v1/interview-prep/:project_id/trigger
   * Enqueues a prep generation job. Returns 202 Accepted.
   * Returns 422 if project has no current resume.
   */
  triggerInterviewPrep: (projectId: string) =>
    apiFetch<void>(`/api/v1/interview-prep/${projectId}/trigger`, {
      method: "POST",
    }),

  /**
   * GET /api/v1/interview-prep/:project_id/status
   * Lightweight status poll — returns {status, last_generated_at, expires_at, is_stale}.
   */
  getInterviewPrepStatus: (projectId: string) =>
    apiFetch<PrepStatusResponse>(`/api/v1/interview-prep/${projectId}/status`),

  /**
   * PUT /api/v1/interview-prep/:project_id/company
   * Update company context and re-run gap questions (STAR scaffolds reused).
   */
  updateInterviewPrepCompany: (projectId: string, body: UpdateCompanyRequest) =>
    apiFetch<PrepMeta | null>(`/api/v1/interview-prep/${projectId}/company`, {
      method: "PUT",
      body: JSON.stringify(body),
    }),
};

export type { UserProfileResponse, UpsertProfileRequest, ProfileLinkData };
