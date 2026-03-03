import type {
  GenerateResponse,
  ResumeDetailResponse,
  AuditManifest,
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
   */
  getRenderStatus: (jobId: string) =>
    apiFetch<{ status: string }>(`/api/v1/render/${jobId}/status`),

  /**
   * Returns the URL to stream the rendered PDF.
   * GET /api/v1/render/:job_id
   */
  getPdfUrl: (jobId: string) => `${API_BASE}/api/v1/render/${jobId}`,
};
