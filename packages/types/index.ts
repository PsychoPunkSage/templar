// Shared TypeScript types for the Templar platform.
// These types mirror the Rust backend data models exactly.
// DO NOT change these independently of the Rust structs in apps/api/src/.

// ─────────────────────────────────────────────────────────────────────────────
// Layout / Generation types
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A resume bullet after layout simulation.
 * Mirrors: apps/api/src/layout/simulator.rs — SimulatedBullet
 */
export interface SimulatedBullet {
  text: string
  source_entry_id: string
  section: string
  /** Line count as measured by the simulator (1 or 2 for passing bullets). */
  verified_line_count: number
  jd_keywords_used: string[]
  /** True if the simulator called the LLM at least once to adjust this bullet. */
  was_adjusted: boolean
  /** True if the bullet still violates the contract after all simulation passes. */
  flagged_for_review: boolean
  /**
   * Page number (1-based) assigned by the CV paginator.
   * Always 1 for single-page resumes. Added in migration 016.
   */
  page_number: number
}

// ─────────────────────────────────────────────────────────────────────────────
// Database row types
// ─────────────────────────────────────────────────────────────────────────────

export interface ProfileLinkData {
  type: string;
  label?: string;
  url: string;
  alias?: string;
}

export interface UpsertProfileRequest {
  user_id: string;
  full_name?: string;
  email?: string;
  phone?: string;
  location?: string;
  links?: ProfileLinkData[];
}

export interface UserProfileResponse {
  full_name: string;
  email: string;
  phone: string;
  location: string;
  links: ProfileLinkData[];
}

/**
 * A resume bullet row from the database.
 * Mirrors: apps/api/src/models/resume.rs — ResumeBulletRow
 */
export interface ResumeBulletRow {
  id: string
  resume_id: string
  section: string
  bullet_text: string
  source_entry_id: string
  grounding_score: number
  is_user_edited: boolean
  line_count: number
  /** Added in migration 010: pre-formatted LaTeX entry header (e.g. `\job{Co}{Role}{Date}`).
   *  Non-null only for the first bullet of each source_entry_id group. */
  entry_header: string | null
  /** Added in migration 011: insertion rank (0-based) for render ordering. */
  order_idx: number
  created_at: string
  /** Set when grounding score failed — from migration 007. */
  rejection_reason: string | null
  /**
   * Page number (1-based) assigned by the CV paginator.
   * Always 1 for single-page resumes. Added in migration 016.
   * May be undefined for pre-migration rows fetched from the DB before schema upgrade.
   */
  page_number?: number
}

/**
 * A resume row from the database.
 * Mirrors: apps/api/src/models/resume.rs — ResumeRow
 */
export interface ResumeRow {
  id: string
  user_id: string
  jd_text: string
  jd_parsed: unknown
  fit_score: number | null
  latex_source: string | null
  s3_pdf_key: string | null
  status: string
  /**
   * 'single_page' | 'cv'. Added in migration 016. Defaults to 'single_page' via
   * #[sqlx(default)] in Rust — always present in API responses.
   */
  resume_type: string
  /**
   * Total page count for CV-mode resumes. Null for single-page resumes or until
   * generation has been completed. Added in migration 016.
   */
  page_count: number | null
  created_at: string
  updated_at: string
}

// ─────────────────────────────────────────────────────────────────────────────
// Fit scoring types
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A keyword extracted from the job description.
 * Mirrors: apps/api/src/generation/jd_parser.rs — KeywordEntry
 */
export interface KeywordEntry {
  keyword: string
  frequency: number
  position_weight: number
  weighted_score: number
}

/**
 * A single matched dimension between user context and a JD keyword/requirement.
 * Mirrors: apps/api/src/generation/fit_scoring.rs — FitMatch
 */
export interface FitMatch {
  dimension: string
  context_evidence: string
  jd_requirement: string
  /** 0.0 – 1.0 */
  strength: number
}

/**
 * A JD keyword or requirement not covered by any context entry.
 * Mirrors: apps/api/src/generation/fit_scoring.rs — Gap
 */
export interface Gap {
  keyword: string
  jd_frequency: number
  suggestion: string | null
}

/**
 * Full fit report returned by the fit scorer.
 * Mirrors: apps/api/src/generation/fit_scoring.rs — FitReport
 *
 * NOTE: This is NOT the same as the spec document describes.
 * The actual backend returns strong_matches/partial_matches/gaps, not
 * matched_keywords/missing_keywords/coverage_percentage.
 */
export interface FitReport {
  /** 0 – 100 */
  overall_score: number
  /** Matches with strength >= 0.8 */
  strong_matches: FitMatch[]
  /** Matches with strength 0.4 – 0.79 */
  partial_matches: FitMatch[]
  /** JD requirements with no matching context entry */
  gaps: Gap[]
  recommendation: string
  /** "keyword" | "llm" — for transparency */
  scorer_backend: string
}

// ─────────────────────────────────────────────────────────────────────────────
// Grounding / Audit types
// ─────────────────────────────────────────────────────────────────────────────

/**
 * One entry in the audit manifest — one per bullet in the resume.
 * Mirrors: apps/api/src/grounding/types.rs — AuditEntry
 */
export interface AuditEntry {
  bullet_text: string
  source_entry_id: string
  composite_score: number
  /** "pass" | "flag_for_review" | "fail" */
  verdict: string
  rejection_reason: string | null
  section: string
}

/**
 * Complete audit manifest for a generated resume.
 * Mirrors: apps/api/src/grounding/types.rs — AuditManifest
 */
export interface AuditManifest {
  resume_id: string
  generated_at: string
  entries: AuditEntry[]
  /** Fraction of bullets that passed (Pass verdict / total). */
  overall_pass_rate: number
  /** Count of bullets with Fail verdict. */
  bullets_rejected: number
  /** Count of bullets with FlagForReview verdict. */
  bullets_flagged: number
}

// ─────────────────────────────────────────────────────────────────────────────
// API response types
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Response from POST /api/v1/resumes/generate (FIX-08).
 * Returns immediately — the actual pipeline runs in the background worker.
 * Mirrors: apps/api/src/generation/handlers.rs — GenerateJobResponse
 */
export interface GenerateJobResponse {
  job_id: string
  /** Always "queued" on a successful enqueue. */
  status: string
}

// ─────────────────────────────────────────────────────────────────────────────
// FIX-10: Entry group types (structured per-entry output for the frontend editor)
// Mirrors: apps/api/src/generation/generator.rs — EntryDisplayHeader / EntryGroup
// ─────────────────────────────────────────────────────────────────────────────

export interface EntryDisplayHeaderExperience {
  type: 'experience'; company: string; role: string; date_range: string;
}
export interface EntryDisplayHeaderProject {
  type: 'project'; name: string; tech_stack: string; date_range: string;
}
export interface EntryDisplayHeaderOpenSource {
  type: 'open_source'; project_name: string; tech_stack: string;
}
export interface EntryDisplayHeaderEducation {
  type: 'education'; institution: string; degree: string; date_range: string;
}
export interface EntryDisplayHeaderSkills {
  type: 'skills'; category: string;
}
export interface EntryDisplayHeaderOther {
  type: 'other'; label: string;
}

export type EntryDisplayHeader =
  | EntryDisplayHeaderExperience
  | EntryDisplayHeaderProject
  | EntryDisplayHeaderOpenSource
  | EntryDisplayHeaderEducation
  | EntryDisplayHeaderSkills
  | EntryDisplayHeaderOther;

/**
 * All content bullets for one context entry, with human-readable display fields
 * for the editor header row (company / role / dates above the bullet list).
 * Mirrors: apps/api/src/generation/generator.rs — EntryGroup
 */
export interface EntryGroup {
  source_entry_id: string
  section: string
  /** Typed display fields for the editor UI — no LaTeX parsing needed. */
  display_header: EntryDisplayHeader
  /** Pre-formatted LaTeX header macro — passed to render pipeline unchanged. */
  entry_header_latex: string | null
  bullets: SimulatedBullet[]
}

/**
 * Response from GET /api/v1/generation/jobs/:id/status (FIX-08 + FIX-10).
 * - queued | processing: no result fields populated
 * - done: entry_groups, fit_report, layout_flagged populated
 * - failed: error populated
 * Mirrors: apps/api/src/generation/handlers.rs — GenerationStatusResponse
 */
export interface GenerationStatusResponse {
  job_id: string
  status: 'queued' | 'processing' | 'done' | 'failed'
  error?: string | null
  resume_id?: string | null
  fit_report?: FitReport | null
  entry_groups?: EntryGroup[] | null
  layout_flagged?: boolean | null
  /**
   * Number of pages in the generated resume (1 for single-page, 1+ for CV mode).
   * Populated only when status='done'. Added in migration 016.
   */
  page_count?: number | null
}

/**
 * @deprecated Use GenerationStatusResponse + GenerateJobResponse instead (FIX-08).
 * Kept for reference; no longer returned by the generate endpoint.
 * Mirrors: apps/api/src/generation/generator.rs — GenerateResponse
 */
export interface GenerateResponse {
  resume_id: string
  fit_report: FitReport
  bullets: SimulatedBullet[]
  status: string
  entry_groups: EntryGroup[]
}

/**
 * Response from POST /api/v1/resumes/fit-score.
 * Mirrors: apps/api/src/generation/handlers.rs — FitScoreResponse
 */
export interface FitScoreResponse {
  fit_report: FitReport
  parsed_jd: unknown
  cache_hit: boolean
  jd_hash: string
  context_hash: string
}

/**
 * Response from GET /api/v1/resumes/:id.
 * Mirrors: apps/api/src/generation/handlers.rs — ResumeDetailResponse
 */
export interface ResumeDetailResponse {
  resume: ResumeRow
  bullets: ResumeBulletRow[]
  /**
   * Populated for resumes generated post-migration 014 — typed display headers for
   * the frontend editor (company/role/dates). Null for legacy resumes; frontend falls
   * back to bulletRowsToEntryGroups() which uses entry_header LaTeX as label.
   */
  entry_groups: EntryGroup[] | null
}

// ─────────────────────────────────────────────────────────────────────────────
// Template types
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A single template summary returned by GET /api/v1/templates.
 * Mirrors: apps/api/src/templates/mod.rs — TemplateSummary
 */
export interface TemplateSummary {
  id: string
  name: string
  description: string
  tags: string[]
  /** API-relative path: `/api/v1/templates/{id}/preview` — prepend API_BASE for use in <img> */
  thumbnail_url: string
}

export interface TemplateListResponse {
  templates: TemplateSummary[]
}

// ─────────────────────────────────────────────────────────────────────────────
// CV Project types
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A CV project row from the database.
 * Mirrors: apps/api/src/projects/mod.rs — CvProjectRow
 */
export interface CvProject {
  id: string
  user_id: string
  name: string
  template_id: string
  /** null until first generation; SET NULL if resume is deleted */
  current_resume_id: string | null
  /** Last job description text entered for this project. Null until first JD is typed. */
  last_jd_text: string | null
  /**
   * Set when a generation job is enqueued (FIX-08).
   * On page load, the editor probes this job's status to resume polling if in-flight.
   */
  generation_job_id: string | null
  /**
   * Whether this project targets a single-page resume or a multi-page CV.
   * Added in migration 016. Defaults to 'single_page'.
   */
  document_type: 'single_page' | 'cv'
  created_at: string
  updated_at: string
}

export interface ProjectListResponse {
  projects: CvProject[]
}

export interface CreateProjectRequest {
  user_id: string
  name: string
  template_id: string
  /**
   * Whether to create a single-page resume project or a multi-page CV project.
   * Defaults to 'single_page' on the backend if omitted.
   */
  document_type?: 'single_page' | 'cv'
}

export interface UpdateProjectRequest {
  name?: string
  template_id?: string
  current_resume_id?: string
  last_jd_text?: string
  /** Set when a generation job is enqueued — persists job_id for page-reload polling (FIX-08). */
  generation_job_id?: string
}

// ─────────────────────────────────────────────────────────────────────────────
// Persona types
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A persona row from the database.
 * Mirrors: apps/api/src/models/resume.rs — PersonaRow
 */
export interface Persona {
  id: string
  user_id: string
  name: string
  emphasized_tags: string[]
  suppressed_tags: string[]
  tone_preference: string | null
  section_order: unknown | null
  created_at: string
}

export interface PersonaListResponse {
  personas: Persona[]
}

export interface CreatePersonaRequest {
  user_id: string
  name: string
  emphasized_tags?: string[]
  suppressed_tags?: string[]
  tone_preference?: string
}

export interface UpdatePersonaRequest {
  name?: string
  emphasized_tags?: string[]
  suppressed_tags?: string[]
  tone_preference?: string | null
}

export interface PersonaSuggestion {
  name: string
  emphasized_tags: string[]
  suppressed_tags: string[]
  /** "startup" | "enterprise" | "research" | "product" | null */
  tone_preference: string | null
  reasoning: string
}

/**
 * Response from GET /api/v1/personas/suggest.
 * Includes content hashes for client-side cache freshness detection.
 * Mirrors: apps/api/src/personas/mod.rs — SuggestPersonasResponse
 */
export interface SuggestPersonasResponse {
  suggestions: PersonaSuggestion[]
  /** SHA-256 of the user's current context entries. */
  context_hash: string
  /** SHA-256 of the user's current persona IDs. */
  persona_hash: string
}

// ─────────────────────────────────────────────────────────────────────────────
// Context Library types
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A context entry row from the database (latest version per entry_id).
 * Mirrors: apps/api/src/models/context.rs — ContextEntryRow
 */
export interface ContextEntryRow {
  /** DB row UUID (unique per version). */
  id: string
  /** Stable logical ID — same across all versions of the same entry. */
  entry_id: string
  version: number
  /** "experience" | "education" | "project" | "skill" | "certification" | "publication" | "open_source" | "award" | "extracurricular" */
  entry_type: string
  /** Structured JSON data for the entry (company, role, bullets, etc.). May be null for raw-text-only entries. */
  data: Record<string, unknown> | null
  raw_text: string | null
  /** 0.0–1.0 recency score (18-month half-life decay). */
  recency_score: number
  /** 0.0–1.0 impact score from validation pass. */
  impact_score: number
  tags: string[]
  flagged_evergreen: boolean
  /** "sole_author" | "primary_contributor" | "team_member" | "reviewer" */
  contribution_type: string
  /** Phase 5.5 non-blocking quality score (0.0–1.0). */
  quality_score: number
  /** Machine-readable quality flags, e.g. ["missing_metric"]. */
  quality_flags: string[]
  created_at: string
}

/**
 * Health report for one context section.
 * Mirrors: apps/api/src/context/completeness.rs — SectionHealth
 */
export interface CompletenessSection {
  section: string
  /** 0.0–1.0 combined recency * impact score. */
  score: number
  entry_count: number
  missing_quantification: number
  /** "strong" | "moderate" | "weak" | "missing" */
  status: 'strong' | 'moderate' | 'weak' | 'missing'
  recommendations: string[]
}

/**
 * Full completeness report returned alongside context entries.
 * Mirrors: apps/api/src/context/completeness.rs — CompletenessReport
 */
export interface CompletenessReport {
  /** 0.0–1.0 weighted across all sections. */
  overall_score: number
  sections: CompletenessSection[]
  total_entries: number
  missing_sections: string[]
}

/**
 * Response from GET /api/v1/context?user_id=...
 * Mirrors: apps/api/src/context/handlers.rs — ContextListResponse
 */
export interface ContextEntriesResponse {
  entries: ContextEntryRow[]
  completeness: CompletenessReport
}
