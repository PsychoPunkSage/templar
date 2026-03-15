import { redirect } from "next/navigation";

// /projects and / are the same page — the root shows the project list.
// This redirect keeps the nav link (/projects) working correctly.
export default function ProjectsRedirect() {
  redirect("/");
}
