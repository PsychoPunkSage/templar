"use client";

import { useRef, useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { ChevronDown, Sparkles } from "lucide-react";
import { cn } from "@/lib/utils";

const MODELS = [
  { id: "claude-sonnet-4-5", label: "Claude Sonnet 4.5", badge: "Default" },
  { id: "claude-opus-4-7", label: "Claude Opus 4.7", badge: "Powerful" },
  { id: "claude-haiku-4-5", label: "Claude Haiku 4.5", badge: "Fast" },
];

interface AnimatedAiInputProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  minRows?: number;
  maxRows?: number;
  className?: string;
  disabled?: boolean;
}

export function AnimatedAiInput({
  value,
  onChange,
  placeholder = "Paste job description here…",
  minRows = 3,
  maxRows = 12,
  className,
  disabled,
}: AnimatedAiInputProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [focused, setFocused] = useState(false);
  const [selectedModel, setSelectedModel] = useState(MODELS[0]);

  useEffect(() => {
    const ta = textareaRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    const lineH = 24;
    const minH = minRows * lineH;
    const maxH = maxRows * lineH;
    const scrollH = ta.scrollHeight;
    ta.style.height = `${Math.min(Math.max(scrollH, minH), maxH)}px`;
    ta.style.overflowY = scrollH > maxH ? "auto" : "hidden";
  }, [value, minRows, maxRows]);

  return (
    <motion.div
      animate={{ borderColor: focused ? "oklch(0.62 0.22 255 / 60%)" : "var(--border)" }}
      transition={{ duration: 0.2 }}
      className={cn(
        "relative rounded-md border bg-card overflow-hidden",
        "transition-shadow duration-200",
        focused && "shadow-[0_0_0_2px_oklch(0.62_0.22_255_/_15%)]",
        className
      )}
    >
      {/* Toolbar */}
      <div className="flex items-center gap-2 px-3 pt-2.5 pb-1.5 border-b border-border">
        <Sparkles className="h-3.5 w-3.5 text-primary shrink-0" />
        <span className="text-xs text-muted-foreground font-medium">Job Description</span>
        <div className="ml-auto">
          <DropdownMenu.Root>
            <DropdownMenu.Trigger asChild>
              <button
                className="flex items-center gap-1 rounded px-2 py-0.5 text-xs text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-colors"
                disabled={disabled}
              >
                {selectedModel.label}
                <ChevronDown className="h-3 w-3" />
              </button>
            </DropdownMenu.Trigger>
            <DropdownMenu.Portal>
              <DropdownMenu.Content
                className="z-50 min-w-[200px] rounded-md border border-border bg-popover p-1 shadow-lg text-popover-foreground"
                sideOffset={4}
                align="end"
              >
                {MODELS.map((model) => (
                  <DropdownMenu.Item
                    key={model.id}
                    onSelect={() => setSelectedModel(model)}
                    className="flex items-center justify-between rounded px-2.5 py-1.5 text-xs cursor-pointer hover:bg-accent hover:text-accent-foreground outline-none"
                  >
                    <span>{model.label}</span>
                    <span className="ml-3 text-[10px] text-muted-foreground bg-muted px-1.5 py-0.5 rounded">
                      {model.badge}
                    </span>
                  </DropdownMenu.Item>
                ))}
              </DropdownMenu.Content>
            </DropdownMenu.Portal>
          </DropdownMenu.Root>
        </div>
      </div>

      {/* Auto-resize textarea */}
      <textarea
        ref={textareaRef}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        placeholder={placeholder}
        disabled={disabled}
        className={cn(
          "w-full resize-none bg-transparent px-3 py-2.5 text-sm text-foreground",
          "placeholder:text-muted-foreground focus:outline-none",
          "disabled:opacity-50 disabled:cursor-not-allowed",
          "leading-6"
        )}
        style={{ minHeight: `${minRows * 24}px` }}
      />

      {/* Character count — visible when content is present */}
      <AnimatePresence>
        {value.length > 0 && (
          <motion.div
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 4 }}
            className="flex justify-end px-3 pb-2"
          >
            <span className="text-[10px] text-muted-foreground">
              {value.length.toLocaleString()} chars
            </span>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
}
