# Templar — Web

Next.js 16 (App Router) frontend for the Templar AI resume engine.

## Getting Started

```bash
bun install
bun dev
```

Open [http://localhost:3001](http://localhost:3001).

## Stack

- **Next.js 16** — App Router, TypeScript
- **Tailwind CSS 4** — `@theme inline`, oklch color tokens
- **shadcn/ui** — Radix-based primitives
- **framer-motion** — animated nav pill, AI input transitions
- **Clerk** — authentication
- **Zustand** — client state

## Build & Deploy

```bash
bun run build          # production build
bun run lint           # ESLint
```

Docker image is built via `infra/Dockerfile.web` using `oven/bun:1-alpine` for the build stage.
