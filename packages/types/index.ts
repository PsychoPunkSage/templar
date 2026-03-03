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
}

// ─────────────────────────────────────────────────────────────────────────────
// Database row types
// ─────────────────────────────────────────────────────────────────────────────

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
  created_at: string
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
 * Response from POST /api/v1/resumes/generate.
 * Mirrors: apps/api/src/generation/handlers.rs — GenerateResponse
 */
export interface GenerateResponse {
  resume_id: string
  fit_report: FitReport
  bullets: SimulatedBullet[]
  status: string
}

/**
 * Response from GET /api/v1/resumes/:id.
 * Mirrors: apps/api/src/generation/handlers.rs — ResumeDetailResponse
 */
export interface ResumeDetailResponse {
  resume: ResumeRow
  bullets: ResumeBulletRow[]
}
