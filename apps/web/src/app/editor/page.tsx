import { redirect } from "next/navigation";

/**
 * The old flat editor route (/editor) no longer exists.
 * All editor sessions are now project-scoped at /editor/[projectId].
 * Redirect to the project list so users can select or create a project.
 */
export default function EditorRedirect() {
  redirect("/");
}
