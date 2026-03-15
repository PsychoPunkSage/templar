"use client";

// We use a separate projectStore rather than adding project fields to resumeStore.
// resumeStore owns "the current generation session" (bullets, fit report, render job).
// projectStore owns "the workspace" (which project, which template, project list).
// Mixing them would mean generating a resume resets your project list — wrong coupling.

import { create } from "zustand";
import { api } from "@/lib/api";
import type { CvProject, TemplateSummary } from "@templar/types";

interface ProjectStore {
  // ─── State ─────────────────────────────────────────────────────────────────
  projects: CvProject[];
  currentProject: CvProject | null;
  templates: TemplateSummary[];
  isLoadingProjects: boolean;
  isLoadingTemplates: boolean;
  error: string | null;

  // ─── Actions ───────────────────────────────────────────────────────────────
  loadProjects: (userId: string) => Promise<void>;
  loadProject: (id: string) => Promise<void>;
  loadTemplates: () => Promise<void>;
  createProject: (userId: string, name: string, templateId: string) => Promise<CvProject>;
  /** Hard-deletes a project. Clears currentProject if it matches. */
  deleteProject: (id: string) => Promise<void>;
  /** Links a generated resume to the current project (fire-and-forget). */
  updateProjectResume: (projectId: string, resumeId: string) => void;
  setCurrentProject: (project: CvProject | null) => void;
  clearError: () => void;
}

export const useProjectStore = create<ProjectStore>((set, get) => ({
  projects: [],
  currentProject: null,
  templates: [],
  isLoadingProjects: false,
  isLoadingTemplates: false,
  error: null,

  setCurrentProject: (project) => set({ currentProject: project }),
  clearError: () => set({ error: null }),

  loadProjects: async (userId) => {
    set({ isLoadingProjects: true, error: null });
    try {
      const { projects } = await api.listProjects(userId);
      set({ projects });
    } catch (e) {
      set({ error: e instanceof Error ? e.message : "Failed to load projects" });
    } finally {
      set({ isLoadingProjects: false });
    }
  },

  loadProject: async (id) => {
    try {
      const project = await api.getProject(id);
      set({ currentProject: project });
    } catch (e) {
      set({ error: e instanceof Error ? e.message : "Failed to load project" });
    }
  },

  loadTemplates: async () => {
    set({ isLoadingTemplates: true, error: null });
    try {
      const { templates } = await api.listTemplates();
      if (templates.length === 0) {
        console.warn("[projectStore] loadTemplates: API returned 0 templates — template selector will be empty");
      } else {
        console.log("[projectStore] loadTemplates: loaded", templates.length, "templates");
      }
      set({ templates });
    } catch (e) {
      set({ error: e instanceof Error ? e.message : "Failed to load templates" });
    } finally {
      set({ isLoadingTemplates: false });
    }
  },

  createProject: async (userId, name, templateId) => {
    const project = await api.createProject({ user_id: userId, name, template_id: templateId });
    // Prepend to list so it appears at the top (projects are ordered by updated_at DESC)
    set((s) => ({ projects: [project, ...s.projects] }));
    return project;
  },

  deleteProject: async (id) => {
    await api.deleteProject(id);
    set((s) => ({
      projects: s.projects.filter((p) => p.id !== id),
      currentProject: s.currentProject?.id === id ? null : s.currentProject,
    }));
  },

  updateProjectResume: (projectId, resumeId) => {
    // We link the resume to the project with a fire-and-forget PATCH call.
    // It's acceptable here because: if it fails, the user still has their generated
    // resume (resume_id is returned to the UI immediately). The project's
    // current_resume_id is "best effort" bookkeeping — not critical path.
    api.updateProject(projectId, { current_resume_id: resumeId }).then((updated) => {
      set((s) => ({
        projects: s.projects.map((p) => (p.id === projectId ? updated : p)),
        currentProject: s.currentProject?.id === projectId ? updated : s.currentProject,
      }));
    }).catch((err) => {
      console.warn("Failed to link resume to project (non-fatal):", err);
    });
  },
}));
