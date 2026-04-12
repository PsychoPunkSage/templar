-- Migration 013: persist generation_job_id on cv_projects
--
-- Allows the frontend to resume polling an in-flight generation job
-- after a page navigation or browser refresh. The editor page reads
-- generation_job_id from the project on load and probes the status endpoint.

ALTER TABLE cv_projects ADD COLUMN IF NOT EXISTS generation_job_id UUID;
