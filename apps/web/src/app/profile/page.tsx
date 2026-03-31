"use client";

import { useEffect, useState } from "react";
import { useProfileStore } from "@/store/profileStore";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import type { ProfileLinkData } from "@templar/types";
import { X, Plus } from "lucide-react";

const LINK_TYPES = ["LinkedIn", "GitHub", "GitLab", "Twitter", "Portfolio", "Custom"] as const;

const LINK_TYPE_COLORS: Record<string, string> = {
  LinkedIn: "bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400",
  GitHub: "bg-gray-100 text-gray-700 dark:bg-gray-800 dark:text-gray-300",
  GitLab: "bg-orange-100 text-orange-700 dark:bg-orange-900/30 dark:text-orange-400",
  Twitter: "bg-sky-100 text-sky-700 dark:bg-sky-900/30 dark:text-sky-400",
  Portfolio: "bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400",
  Custom: "bg-muted text-muted-foreground",
};

export default function ProfilePage() {
  const { profile, isLoading, isSaving, error, loadProfile, saveProfile, clearError } = useProfileStore();

  // Local form state — initialized from store once loaded
  const [fullName, setFullName] = useState("");
  const [email, setEmail] = useState("");
  const [phone, setPhone] = useState("");
  const [location, setLocation] = useState("");
  const [links, setLinks] = useState<ProfileLinkData[]>([]);

  // Add-link form state
  const [showAddLink, setShowAddLink] = useState(false);
  const [newLinkType, setNewLinkType] = useState<string>("LinkedIn");
  const [newLinkLabel, setNewLinkLabel] = useState("");
  const [newLinkUrl, setNewLinkUrl] = useState("");
  const [newLinkAlias, setNewLinkAlias] = useState("");

  useEffect(() => {
    loadProfile();
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Sync local state when profile loads
  useEffect(() => {
    if (profile) {
      setFullName(profile.full_name);
      setEmail(profile.email);
      setPhone(profile.phone);
      setLocation(profile.location);
      setLinks(profile.links ?? []);
    }
  }, [profile]);

  const handleSave = async () => {
    await saveProfile({ full_name: fullName, email, phone, location, links });
  };

  const handleAddLink = () => {
    if (!newLinkUrl.trim()) return;
    const link: ProfileLinkData = {
      type: newLinkType,
      url: newLinkUrl.trim(),
      alias: newLinkAlias.trim() || undefined,
      label: newLinkType === "Custom" ? newLinkLabel.trim() || undefined : undefined,
    };
    setLinks((prev) => [...prev, link]);
    setNewLinkType("LinkedIn");
    setNewLinkLabel("");
    setNewLinkUrl("");
    setNewLinkAlias("");
    setShowAddLink(false);
  };

  const handleRemoveLink = (index: number) => {
    setLinks((prev) => prev.filter((_, i) => i !== index));
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-[60vh] text-muted-foreground text-sm">
        Loading profile...
      </div>
    );
  }

  return (
    <div className="max-w-xl mx-auto py-10 px-4">
      <h1 className="text-2xl font-bold mb-1">Profile</h1>
      <p className="text-sm text-muted-foreground mb-6">
        Your contact information and links used in resume headers.
      </p>

      {error && (
        <div className="mb-4 px-3 py-2 bg-destructive/10 text-destructive text-sm rounded-md flex items-center justify-between">
          <span>{error}</span>
          <button onClick={clearError} className="font-bold ml-4 hover:opacity-70">×</button>
        </div>
      )}

      <Card className="p-6 flex flex-col gap-5">
        {/* Contact info */}
        <div className="flex flex-col gap-4">
          <h2 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">
            Contact Info
          </h2>
          <div className="grid grid-cols-2 gap-3">
            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium">Full Name</label>
              <Input value={fullName} onChange={(e) => setFullName(e.target.value)} placeholder="Alex Johnson" />
            </div>
            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium">Email</label>
              <Input type="email" value={email} onChange={(e) => setEmail(e.target.value)} placeholder="alex@example.com" />
            </div>
            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium">Phone</label>
              <Input value={phone} onChange={(e) => setPhone(e.target.value)} placeholder="+1 555 000 0000" />
            </div>
            <div className="flex flex-col gap-1.5">
              <label className="text-xs font-medium">Location</label>
              <Input value={location} onChange={(e) => setLocation(e.target.value)} placeholder="San Francisco, CA" />
            </div>
          </div>
        </div>

        <Separator />

        {/* Links */}
        <div className="flex flex-col gap-3">
          <h2 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">
            Links
          </h2>
          <p className="text-xs text-muted-foreground -mt-1">
            Added links appear in the resume header. The alias is shown instead of the raw URL.
          </p>

          {links.length > 0 && (
            <div className="flex flex-col gap-2">
              {links.map((link, i) => (
                <div key={i} className="flex items-center gap-2 rounded-md border px-3 py-2 text-sm">
                  <span className={`shrink-0 rounded-full px-2 py-0.5 text-xs font-medium ${LINK_TYPE_COLORS[link.type] ?? LINK_TYPE_COLORS.Custom}`}>
                    {link.type === "Custom" && link.label ? link.label : link.type}
                  </span>
                  <span className="flex-1 truncate text-muted-foreground text-xs">{link.url}</span>
                  {link.alias && (
                    <span className="shrink-0 text-xs font-medium">{link.alias}</span>
                  )}
                  <button
                    onClick={() => handleRemoveLink(i)}
                    className="shrink-0 text-muted-foreground hover:text-destructive transition-colors"
                    aria-label="Remove link"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </div>
              ))}
            </div>
          )}

          {showAddLink ? (
            <div className="rounded-md border p-3 flex flex-col gap-3">
              <div className="flex items-center gap-2">
                <select
                  value={newLinkType}
                  onChange={(e) => setNewLinkType(e.target.value)}
                  className="h-9 rounded-md border border-input bg-background px-3 text-sm focus:outline-none focus:ring-2 focus:ring-ring"
                >
                  {LINK_TYPES.map((t) => (
                    <option key={t} value={t}>{t}</option>
                  ))}
                </select>
                {newLinkType === "Custom" && (
                  <Input
                    value={newLinkLabel}
                    onChange={(e) => setNewLinkLabel(e.target.value)}
                    placeholder="Label (e.g. Blog)"
                    className="flex-1"
                  />
                )}
              </div>
              <Input
                value={newLinkUrl}
                onChange={(e) => setNewLinkUrl(e.target.value)}
                placeholder="URL (e.g. https://github.com/username)"
              />
              <Input
                value={newLinkAlias}
                onChange={(e) => setNewLinkAlias(e.target.value)}
                placeholder="Alias — shown in header (e.g. PsychoPunkSage)"
              />
              <div className="flex gap-2">
                <Button size="sm" onClick={handleAddLink} disabled={!newLinkUrl.trim()}>
                  Add
                </Button>
                <Button size="sm" variant="outline" onClick={() => setShowAddLink(false)}>
                  Cancel
                </Button>
              </div>
            </div>
          ) : (
            <Button
              variant="outline"
              size="sm"
              className="self-start"
              onClick={() => setShowAddLink(true)}
            >
              <Plus className="h-3.5 w-3.5 mr-1.5" />
              Add Link
            </Button>
          )}
        </div>

        <Separator />

        <Button onClick={handleSave} disabled={isSaving} className="w-full">
          {isSaving ? "Saving..." : "Save Profile"}
        </Button>
      </Card>
    </div>
  );
}
