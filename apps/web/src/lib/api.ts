import type {
  GenerateResponse,
  ResumeDetailResponse,
  AuditManifest,
  TemplateListResponse,
  CvProject,
  ProjectListResponse,
  CreateProjectRequest,
  UpdateProjectRequest,
  ContextEntriesResponse,
} from "@templar/types";

export type {
  ContextEntryRow,
  CompletenessReport,
  CompletenessSection,
  ContextEntriesResponse,
} from "@templar/types";

const API_BASE =
  process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";

async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    headers: { "Content-Type": "application/json" },
    ...init,
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({})) as Record<string, unknown>;
    const errorMsg =
      (err?.error as Record<string, unknown>)?.message ??
      `HTTP ${res.status}`;
    throw new Error(String(errorMsg));
  }
  return res.json() as Promise<T>;
}

export const api = {
  /**
   * POST /api/v1/resumes/generate
   * Generates a resume from context entries + job description.
   */
  generateResume: (userId: string, jdText: string) =>
    apiFetch<GenerateResponse>("/api/v1/resumes/generate", {
      method: "POST",
      body: JSON.stringify({ user_id: userId, jd_text: jdText }),
    }),

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
};
