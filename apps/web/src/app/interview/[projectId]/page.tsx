"use client";

import { useParams } from "next/navigation";
import { InterviewPrepPage } from "@/components/interview/InterviewPrepPage";

export default function InterviewPage() {
  const params = useParams();
  const projectId = params.projectId as string;

  return <InterviewPrepPage projectId={projectId} />;
}
