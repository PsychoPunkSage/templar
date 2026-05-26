"use client";

import { useEffect } from "react";
import Link from "next/link";
import { motion } from "framer-motion";
import { Plus, Shield, Cpu, Layers } from "lucide-react";
import { useProjectStore } from "@/store/projectStore";
import { useAuthStore } from "@/store/authStore";
import { MVP_USER_ID } from "@/store/resumeStore";
import { ShaderBackground } from "@/components/ui/shader-background";
import { HoverButton } from "@/components/ui/hover-button";
import { Md3Button } from "@/components/ui/material-design-3-button";
import { ProjectCard } from "@/components/ui/project-card";

function formatDate(iso: string) {
  return new Date(iso).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

const STATS = [
  { value: "≥0.80", label: "Grounding score" },
  { value: "3-pass", label: "Layout simulation" },
  { value: "5", label: "Templates" },
  { value: "0", label: "Hallucinations" },
];

const FEATURES = [
  {
    icon: Cpu,
    title: "Context Engine",
    description:
      "Versioned professional data with recency decay, impact scoring, and mandatory validation. Every claim traceable.",
  },
  {
    icon: Layers,
    title: "Layout Optimizer",
    description:
      "Line-fill simulation loop with hard 2-line constraints. Font-aware, sub-3-pass, no overflow tolerance.",
  },
  {
    icon: Shield,
    title: "Grounding System",
    description:
      "Every bullet scored against context entries. Scope inflation forbidden. Below 0.80 — rejected, never shown.",
  },
];

// ── Animations ──────────────────────────────────────────────────────────────

const fadeUp = {
  hidden: { opacity: 0, y: 22 },
  show: { opacity: 1, y: 0 },
};

const staggerContainer = {
  hidden: {},
  show: { transition: { staggerChildren: 0.1 } },
};

const cardVariant = {
  hidden: { opacity: 0, y: 28 },
  show: { opacity: 1, y: 0, transition: { duration: 0.45, ease: "easeOut" as const } },
};

// ── Skeleton ─────────────────────────────────────────────────────────────────

function ProjectSkeleton() {
  return (
    <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
      {[0, 1, 2].map((i) => (
        <div
          key={i}
          className="rounded-2xl border border-border overflow-hidden animate-pulse bg-card"
          style={{ animationDelay: `${i * 120}ms` }}
        >
          {/* Preview area */}
          <div className="h-48 bg-muted/30" />
          {/* Content area */}
          <div className="p-4 space-y-3">
            <div className="h-4 bg-muted rounded w-3/4" />
            <div className="h-3 bg-muted rounded w-1/3" />
            <div className="flex justify-between">
              <div className="h-3 bg-muted rounded w-1/4" />
              <div className="h-3 bg-muted rounded w-1/6" />
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

// ── Page ─────────────────────────────────────────────────────────────────────

export default function HomePage() {
  const userId = useAuthStore((s) => s.internalUserId) ?? MVP_USER_ID;
  const { projects, isLoadingProjects, loadProjects, deleteProject } = useProjectStore();

  useEffect(() => {
    loadProjects(userId);
  }, [loadProjects, userId]);

  const hasProjects = !isLoadingProjects && projects.length > 0;
  const isEmpty = !isLoadingProjects && projects.length === 0;

  return (
    <div className="relative min-h-[calc(100vh-53px)]">
      {/* ── SHADER — always visible full-page background ──────────────── */}
      <ShaderBackground className="fixed inset-0 w-full h-full -z-10" intensity={0.85} />

      {/* ── HERO ─────────────────────────────────────────────────────────── */}
      {isEmpty && (
        <div className="relative flex flex-col items-center justify-center text-center px-6 py-24 overflow-hidden min-h-[52vh]">

          {/* Gradient fade at bottom */}
          <div className="absolute bottom-0 inset-x-0 h-32 bg-gradient-to-t from-black/60 to-transparent pointer-events-none" />

          <motion.div
            className="relative z-10 max-w-xl"
            variants={staggerContainer}
            initial="hidden"
            animate="show"
          >
            <motion.p
              variants={fadeUp}
              className="text-xs font-mono tracking-[0.25em] uppercase text-primary mb-4"
            >
              — AI Resume Engine —
            </motion.p>

            <motion.h1
              variants={fadeUp}
              className="text-4xl sm:text-5xl font-mono font-bold tracking-tight leading-[1.1] mb-6"
              style={{ letterSpacing: "-0.035em" }}
            >
              Build resumes that
              <br />
              <span className="text-primary">defend themselves.</span>
            </motion.h1>

            <motion.p
              variants={fadeUp}
              className="text-sm text-muted-foreground leading-relaxed mb-8 max-w-sm mx-auto"
            >
              Every bullet traceable to verified context. Every line layout-optimized.
              Zero hallucinations.
            </motion.p>

            <motion.div variants={fadeUp}>
              <Link href="/projects/new">
                <HoverButton className="px-7 py-3 text-sm">
                  <Plus className="h-4 w-4" />
                  Create first project
                </HoverButton>
              </Link>
            </motion.div>
          </motion.div>
        </div>
      )}

      {/* ── STATS STRIP ──────────────────────────────────────────────────── */}
      {isEmpty && (
        <motion.div
          variants={staggerContainer}
          initial="hidden"
          whileInView="show"
          viewport={{ once: true }}
          className="border-y border-border/60 py-6 px-6"
        >
          <div className="max-w-3xl mx-auto grid grid-cols-2 sm:grid-cols-4 gap-6 text-center">
            {STATS.map((s) => (
              <motion.div key={s.label} variants={fadeUp} className="flex flex-col gap-1">
                <span className="font-mono font-bold text-2xl text-primary">{s.value}</span>
                <span className="text-xs text-muted-foreground">{s.label}</span>
              </motion.div>
            ))}
          </div>
        </motion.div>
      )}

      {/* ── FEATURE CARDS ────────────────────────────────────────────────── */}
      {isEmpty && (
        <motion.div
          variants={staggerContainer}
          initial="hidden"
          whileInView="show"
          viewport={{ once: true, margin: "-80px" }}
          className="max-w-4xl mx-auto px-6 py-14 grid sm:grid-cols-3 gap-4"
        >
          {FEATURES.map((f) => (
            <motion.div
              key={f.title}
              variants={cardVariant}
              whileHover={{ y: -5, scale: 1.02 }}
              transition={{ type: "spring", stiffness: 350, damping: 26 }}
              className="rounded-xl border border-border bg-card p-5 flex flex-col gap-3 cursor-default"
            >
              <div className="h-9 w-9 rounded-lg bg-primary/10 flex items-center justify-center">
                <f.icon className="h-4 w-4 text-primary" />
              </div>
              <div>
                <p className="text-xs font-mono tracking-[0.15em] uppercase text-primary mb-1">
                  {f.title}
                </p>
                <p className="text-sm text-muted-foreground leading-relaxed">{f.description}</p>
              </div>
            </motion.div>
          ))}
        </motion.div>
      )}

      {/* ── PROJECTS LIST ────────────────────────────────────────────────── */}
      {(hasProjects || isLoadingProjects) && (
        <div className="max-w-5xl mx-auto px-6 py-10 relative z-10">
          <div className="flex items-center justify-between mb-7">
            <div>
              <p className="text-xs font-mono tracking-[0.2em] uppercase text-primary mb-1">
                — Dashboard
              </p>
              <h1
                className="text-xl font-mono font-semibold tracking-tight"
                style={{ letterSpacing: "-0.02em" }}
              >
                Projects
              </h1>
              {hasProjects && (
                <p className="text-xs text-muted-foreground mt-0.5">
                  {projects.length} {projects.length === 1 ? "project" : "projects"}
                </p>
              )}
            </div>
            <Link href="/projects/new">
              <Md3Button variant="outlined" size="sm">
                <Plus className="h-3.5 w-3.5" />
                New Project
              </Md3Button>
            </Link>
          </div>

          {isLoadingProjects ? (
            <ProjectSkeleton />
          ) : (
            <motion.div
              variants={staggerContainer}
              initial="hidden"
              animate="show"
              className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3"
            >
              {projects.map((project) => (
                <ProjectCard
                  key={project.id}
                  project={project}
                  onDelete={deleteProject}
                  variants={cardVariant}
                />
              ))}
            </motion.div>
          )}
        </div>
      )}
    </div>
  );
}
