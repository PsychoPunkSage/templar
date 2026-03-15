"use client";

// Root page — client component that loads the user's project list from the store.
// Auth is not yet wired (Phase 9), so we use the MVP_USER_ID constant.
// Using a client component here avoids Next.js 16 server-component fetch issues
// when the API is not running in dev (shows graceful loading + empty state).

import { useEffect } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useProjectStore } from "@/store/projectStore";

// Hardcoded MVP user — replaced by Clerk auth in Phase 9
const MVP_USER_ID = "00000000-0000-0000-0000-000000000001";

function formatDate(iso: string) {
  return new Date(iso).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

// Loading skeleton — shown while isLoadingProjects === true
function ProjectSkeleton() {
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      {[0, 1, 2, 3].map((i) => (
        <div
          key={i}
          className="rounded-xl border p-5 animate-pulse"
        >
          <div className="flex items-start justify-between gap-3">
            <div className="flex-1 space-y-2">
              <div className="h-4 bg-muted rounded w-3/4" />
              <div className="h-3 bg-muted rounded w-1/2" />
            </div>
            <div className="h-5 w-16 bg-muted rounded-full shrink-0" />
          </div>
          <div className="h-3 bg-muted rounded w-1/3 mt-3" />
        </div>
      ))}
    </div>
  );
}

export default function HomePage() {
  const router = useRouter();
  const { projects, isLoadingProjects, loadProjects, deleteProject } = useProjectStore();

  useEffect(() => {
    loadProjects(MVP_USER_ID);
  }, [loadProjects]);

  return (
    <div className="max-w-4xl mx-auto px-6 py-10">
      {/* Page header */}
      <div className="flex items-center justify-between mb-8">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Your Projects</h1>
          <p className="text-sm text-muted-foreground mt-1">
            Each project links a resume template to your generated resumes.
          </p>
        </div>
        <Link
          href="/projects/new"
          className="inline-flex items-center gap-2 rounded-md bg-primary text-primary-foreground px-4 py-2 text-sm font-medium hover:bg-primary/90 transition-colors"
        >
          New Project
        </Link>
      </div>

      {/* Project list */}
      {isLoadingProjects ? (
        <ProjectSkeleton />
      ) : projects.length === 0 ? (
        <div className="flex flex-col items-center justify-center rounded-xl border border-dashed py-20 text-center gap-4">
          <p className="text-muted-foreground text-sm">No projects yet.</p>
          <Link
            href="/projects/new"
            className="inline-flex items-center gap-2 rounded-md bg-primary text-primary-foreground px-4 py-2 text-sm font-medium hover:bg-primary/90 transition-colors"
          >
            Create your first project
          </Link>
        </div>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2">
          {projects.map((project) => (
            <div
              key={project.id}
              onClick={() => router.push(`/editor/${project.id}`)}
              className="group relative rounded-xl border p-5 hover:border-primary/60 hover:bg-muted/30 transition-all cursor-pointer"
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <h2 className="font-semibold truncate group-hover:text-primary transition-colors">
                    {project.name}
                  </h2>
                  <p className="text-xs text-muted-foreground mt-0.5">
                    Template: {project.template_id}
                  </p>
                </div>
                <div className="flex items-center gap-2 shrink-0">
                  <span
                    className={`rounded-full px-2 py-0.5 text-xs font-medium ${
                      project.current_resume_id
                        ? "bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-300"
                        : "bg-muted text-muted-foreground"
                    }`}
                  >
                    {project.current_resume_id ? "Generated" : "Draft"}
                  </span>
                  <button
                    onClick={(e) => {
                      e.stopPropagation();
                      if (window.confirm(`Delete project "${project.name}"? This cannot be undone.`)) {
                        deleteProject(project.id);
                      }
                    }}
                    className="text-muted-foreground hover:text-destructive transition-colors text-base leading-none px-1 rounded"
                    title="Delete project"
                    aria-label="Delete project"
                  >
                    &#8942;
                  </button>
                </div>
              </div>
              <p className="text-xs text-muted-foreground mt-3">
                Updated {formatDate(project.updated_at)}
              </p>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
