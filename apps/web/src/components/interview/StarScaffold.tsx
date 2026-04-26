"use client";

import type { StarScaffold as StarScaffoldType } from "@templar/types";

interface StarScaffoldProps {
  scaffold: StarScaffoldType;
  bulletText: string;
}

export function StarScaffoldViewer({ scaffold, bulletText }: StarScaffoldProps) {
  return (
    <div className="space-y-4">
      <div className="rounded-md bg-muted/50 px-3 py-2 text-sm italic text-muted-foreground border border-border">
        {bulletText}
      </div>

      <div className="grid gap-3">
        <StarField label="S — Situation" text={scaffold.situation} color="blue" />
        <StarField label="T — Task" text={scaffold.task} color="indigo" />
        <StarField label="A — Action" text={scaffold.action} color="violet" />
        <StarField label="R — Result" text={scaffold.result} color="emerald" />
      </div>

      {scaffold.talking_points.length > 0 && (
        <div>
          <h4 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide mb-2">
            Talking points
          </h4>
          <ul className="space-y-1">
            {scaffold.talking_points.map((point, i) => (
              <li
                key={i}
                className="flex items-start gap-2 text-sm text-foreground"
              >
                <span className="shrink-0 mt-0.5 h-4 w-4 rounded-full bg-indigo-100 dark:bg-indigo-900/40 text-indigo-600 dark:text-indigo-300 text-xs flex items-center justify-center font-medium">
                  {i + 1}
                </span>
                <span>{point}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

type ColorKey = "blue" | "indigo" | "violet" | "emerald";

const colorClasses: Record<ColorKey, string> = {
  blue:    "bg-blue-50 dark:bg-blue-950/30 border-blue-200 dark:border-blue-800",
  indigo:  "bg-indigo-50 dark:bg-indigo-950/30 border-indigo-200 dark:border-indigo-800",
  violet:  "bg-violet-50 dark:bg-violet-950/30 border-violet-200 dark:border-violet-800",
  emerald: "bg-emerald-50 dark:bg-emerald-950/30 border-emerald-200 dark:border-emerald-800",
};

function StarField({ label, text, color }: { label: string; text: string; color: ColorKey }) {
  return (
    <div className={`rounded-md border px-3 py-2 ${colorClasses[color]}`}>
      <p className="text-xs font-semibold text-muted-foreground mb-1">{label}</p>
      <p className="text-sm text-foreground leading-relaxed">{text}</p>
    </div>
  );
}
