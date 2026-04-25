"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { CompanyContext } from "@templar/types";

interface CompanyContextFormProps {
  current?: CompanyContext;
  onSave: (ctx: CompanyContext) => Promise<void>;
}

export function CompanyContextForm({ current, onSave }: CompanyContextFormProps) {
  const [companyName, setCompanyName] = useState(current?.company_name ?? "");
  const [companyStage, setCompanyStage] = useState(current?.company_stage ?? "");
  const [roleTitle, setRoleTitle] = useState(current?.role_title ?? "");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  const handleSave = async () => {
    if (!companyName.trim()) return;
    setSaving(true);
    setSaved(false);
    try {
      await onSave({
        company_name: companyName.trim(),
        company_stage: companyStage.trim() || undefined,
        role_title: roleTitle.trim() || undefined,
      });
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="space-y-3 p-4 rounded-lg border border-border bg-muted/30">
      <h4 className="text-sm font-semibold text-foreground">Company context</h4>
      <p className="text-xs text-muted-foreground">
        Providing company details refines gap questions for this specific role.
      </p>

      <div className="space-y-2">
        <div>
          <label className="text-xs text-muted-foreground mb-1 block">
            Company name
          </label>
          <Input
            value={companyName}
            onChange={(e) => setCompanyName(e.target.value)}
            placeholder="e.g. Acme Corp"
            className="h-8 text-sm"
          />
        </div>
        <div>
          <label className="text-xs text-muted-foreground mb-1 block">
            Company stage (optional)
          </label>
          <Input
            value={companyStage}
            onChange={(e) => setCompanyStage(e.target.value)}
            placeholder="e.g. Series B startup, Fortune 500"
            className="h-8 text-sm"
          />
        </div>
        <div>
          <label className="text-xs text-muted-foreground mb-1 block">
            Role title (optional)
          </label>
          <Input
            value={roleTitle}
            onChange={(e) => setRoleTitle(e.target.value)}
            placeholder="e.g. Senior Software Engineer"
            className="h-8 text-sm"
          />
        </div>
      </div>

      <Button
        size="sm"
        onClick={handleSave}
        disabled={saving || !companyName.trim()}
        className="w-full"
      >
        {saving ? "Saving..." : saved ? "Saved" : "Save and refresh questions"}
      </Button>
    </div>
  );
}
