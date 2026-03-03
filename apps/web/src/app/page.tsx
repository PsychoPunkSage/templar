import { redirect } from "next/navigation";

/**
 * Root page — redirects immediately to the editor.
 * Auth-gating will be added in Phase 9 (Clerk integration).
 */
export default function Home() {
  redirect("/editor");
}
