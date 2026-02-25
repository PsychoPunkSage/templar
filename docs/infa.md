**Frontend**

- Next.js (App Router) + TypeScript
- Tailwind CSS + shadcn/ui
- PDF.js for preview rendering
- Zustand for state management

**Backend**

- Rust (Axum) — you're already strong here, and performance matters for the layout simulation loop
- PostgreSQL (context metadata, versioning, audit manifests)
- S3-compatible storage (raw `.md` context files, generated PDFs)
- Redis (job queues for async LaTeX rendering)

**AI / LLM**

- Claude API (claude-sonnet-4-5) — best instruction-following for constrained structured generation

**LaTeX Rendering**

- Tectonic — Rust-based, self-contained, no TeX Live needed, perfect for server-side rendering in a Rust backend
- Compile to PDF server-side, stream to frontend

**Auth**

- Clerk or Supabase Auth — ship fast, don't build this

**Infrastructure**

- Docker + Railway or Render for MVP
- Upgrade to AWS/GCP when scaling
