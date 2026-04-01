# Templar

Templar is an AI-powered, context-aware, layout-optimized resume generation platform. It is not a formatter or a template marketplace. It reasons from a deep, verified, living user context against a specific job description, then generates a resume constrained by both layout physics and factual integrity.

**North Star:** Every resume Templar generates must be something the user could defend in an interview, line by line, with zero embarrassment.

---

## How It Works

1. **Build your context** — ingest your professional history once. Every entry is validated for specificity and impact at ingestion time.
2. **Paste a job description** — Templar parses it for hard requirements, soft signals, keywords, and tone.
3. **Get a fit report** — see a structured score (strong matches, partial matches, gaps) before any content is generated.
4. **Generate** — a four-step strategy pass selects content, calibrates tone, and produces bullets. Every bullet is grounded to a verified context entry (minimum score 0.80) and enforced against a hard 2-line layout contract.
5. **Render** — LaTeX is compiled server-side via pdflatex and streamed to an in-browser PDF.js preview.

---

## Tech Stack

### Frontend
- Next.js (App Router) + TypeScript
- Tailwind CSS + shadcn/ui
- PDF.js — in-browser PDF preview
- Zustand — client state management

### Backend
- Rust (Axum) — primary API server; runs the layout simulation loop
- PostgreSQL — context metadata, versioning, audit manifests
- S3-compatible storage — raw `.md` context files, generated PDFs
- Redis — async job queue for LaTeX rendering

### AI
- Claude API (`claude-sonnet-4-5`) — context parsing, generation, grounding, compression

### LaTeX Rendering
- pdflatex (TeX Live) — server-side PDF compilation, streamed to frontend
  - Note: switched from Tectonic due to format-cache hash mismatches in containerized environments

### Auth
- Clerk or Supabase Auth

### Infrastructure
- Docker + Railway or Render for MVP

---

## Repository Structure

```
templar/
├── apps/
│   ├── web/              # Next.js frontend
│   └── api/              # Rust/Axum backend
├── packages/
│   ├── db/               # PostgreSQL schema & migrations
│   ├── types/            # Shared TypeScript types
│   └── ui/               # Shared UI components (shadcn extensions)
├── infra/                # Docker, Railway/Render configs
├── docs/
│   ├── spec.md           # Full product specification
│   └── infa.md           # Infrastructure decisions
└── CLAUDE.md
```

---

## Core Systems

### Context Engine (`apps/api/src/context/`)

Stores structured professional data as versioned `.md` snapshots plus relational metadata. Versioning is append-only — no UPDATE of existing rows.

- Every entry carries: `entry_id`, `recency_score`, `impact_score`, `tags`, `flagged_evergreen`, `contribution_type`
- Contribution types: `sole_author`, `primary_contributor`, `team_member`, `reviewer`
- Recency decays on an 18-month configurable half-life
- Combined relevance score: `0.5 * recency + 0.3 * impact + 0.2 * jd_relevance`
- Impact validation is mandatory at ingestion — vague claims are rejected or flagged for quantification

### Generation Engine (`apps/api/src/generation/`)

Four silent strategy steps before any content is written:

1. **JD Parsing** — hard requirements, soft signals, role signals, keyword inventory weighted by position
2. **Fit Scoring** — structured report surfaced to the user before generation (LLM-powered or keyword fallback via `FIT_SCORER_BACKEND` env var)
3. **Content Selection** — rank, select, exclude, and reframe context entries against the JD
4. **Tone Calibration** — startup context produces "Architected"; enterprise context produces "Contributed to"; contribution type is enforced throughout

### Layout Optimization (`apps/api/src/layout/`)

The hardest engineering component. Line coverage is a hard physical constraint, not a style preference.

| Bullet Type | Coverage Requirement |
|---|---|
| 1-line | >= 80% horizontal fill |
| 2-line | Line 1 = 100%, Line 2 >= 70% |
| 3+ lines | Prohibited — compress unconditionally |

**2-line promotion** requires all three: quantified outcome (HIGH), technical depth (HIGH), JD relevance (HIGH). Maximum 3 two-line bullets per page.

**Simulation loop:**
```
LLM draft
  → line-fill simulator (font metric tables + char width tables)
  → violation? → LLM expand/compress with explicit budget
  → re-simulate (max 3 passes)
  → flag for human review on failure
  → pdflatex render
```

Font changes trigger full re-simulation. Simulation runs in `tokio::task::spawn_blocking`.

### Grounding System (`apps/api/src/grounding/`)

Every bullet must be traceable to a verified context entry. Below-threshold bullets are rejected and regenerated — they are never shown to the user.

**Composite score formula:**
```
0.40 * source_match
+ 0.30 * specificity_fidelity
+ 0.20 * scope_accuracy
- 0.10 * interpolation_risk
```

**Hard rules:**
- Minimum score: 0.80
- `team_member` entries cannot produce "Architected", "Led", "Owned", or equivalent language
- Every resume generates a hidden audit manifest in the database (never in the PDF)

### Render Service (`apps/api/src/render/`)

- pdflatex compilation via async Redis job queue — the request thread is never blocked
- Content-hash caching: skips re-render if content is unchanged
- Target: < 2 seconds for section-level re-render

**Required LaTeX packages:** `microtype`, `geometry`, `enumitem`, `hyperref`, `fontspec`, `titlesec`, `xcolor`, `tabularx`, `parskip`, `fontawesome5`

---

## Templates

Five built-in templates, all parameterized (font, spacing, margins injected at render time):

| Template | Use Case |
|---|---|
| Hacker | Software engineering, open source — minimal, dense |
| Researcher | Academia, R&D — formal, publication-ready |
| Operator | PM, leadership — spacious, achievement-forward |
| Founder | Startups, VC-facing — bold headers |
| Classic | ATS-safe — plain, highly parseable |

---

## Getting Started

### Prerequisites

- Docker and Docker Compose
- An Anthropic API key

### Run locally

```bash
# Clone the repo
git clone https://github.com/PsychoPunkSage/templar.git
cd templar

# Copy and fill in environment variables
cp infra/.env.example infra/.env

# Build and start all services (API, web, PostgreSQL, Redis, MinIO)
docker compose -f infra/docker-compose.yml up -d --build api web
```

The web UI will be available at `http://localhost:3000`. The API runs at `http://localhost:8080`.

### Environment Variables

```env
# Claude API
ANTHROPIC_API_KEY=

# Database
DATABASE_URL=

# S3-compatible storage
S3_BUCKET=
S3_ENDPOINT=
AWS_ACCESS_KEY_ID=
AWS_SECRET_ACCESS_KEY=

# Redis
REDIS_URL=

# Auth
CLERK_SECRET_KEY=
NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY=

# App
NEXT_PUBLIC_API_URL=
```

---

## Development Status

| Phase | Description | Status |
|---|---|---|
| 0 | Infra scaffold (monorepo, Docker, CI) | Complete |
| 1 | Context Engine | Complete — 36 tests |
| 2 | Generation Engine | Complete — 78 tests |
| 3 | Layout Optimization | Complete — 122 tests |
| 4 | Render Pipeline | Complete — 143 tests |
| 5 | Grounding / Anti-Hallucination | Complete — 164 tests |
| 5.5 | Context Robustness & Render Fix | Complete — 182 tests |
| 7.0 | LlmFitScorer | Complete |
| 8 | Template System + CV Projects + UI Nav | Complete — 189 tests |
| — | Render Pipeline Unification | Complete — 223 tests |
| — | Render/Generation Decoupling + UI fixes | Complete |
| — | UI/UX Overhaul | Complete |

---

## Key Engineering Constraints

These are architectural laws, not preferences:

- **No bullet may exceed 2 lines** — enforced at layout simulation before any LaTeX is rendered
- **No bullet without a verified source** — grounding score must be >= 0.80; below this, reject and regenerate
- **Scope inflation is forbidden** — `team_member` context entries cannot produce "Architected", "Led", or "Owned" language
- **Font changes trigger full re-simulation** — Garamond vs. Inter produces 5–8% rendered width variance
- **LLM cannot measure rendered width** — the simulation loop must iterate; never trust first-pass LLM output
- **Context versioning is append-only** — existing rows are never updated; every change is a new INSERT

---

## Architecture Notes

- All LLM calls route through a single `llm_client` module — no direct Anthropic SDK calls scattered across services
- The audit manifest (grounding scores, context versions, timestamps) is written to the database and never exposed in the PDF
- `FIT_SCORER_BACKEND=llm` enables LlmFitScorer; the default is KeywordFitScorer (faster, no LLM call required for fit analysis)
- Async batch context ingestion is supported — paste a large document and entries are split, queued via Redis, and processed by background workers

---

## License

MIT
