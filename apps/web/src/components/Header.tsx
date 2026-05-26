"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { UserButton } from "@clerk/nextjs";
import { ThemeToggle } from "@/components/ThemeToggle";
import { NavHeader } from "@/components/ui/nav-header";

export function Header() {
  const [scrolled, setScrolled] = useState(false);

  useEffect(() => {
    const handler = () => setScrolled(window.scrollY > 10);
    window.addEventListener("scroll", handler, { passive: true });
    return () => window.removeEventListener("scroll", handler);
  }, []);

  return (
    <div className="sticky top-0 z-50 shrink-0 flex justify-center px-6 py-3 pointer-events-none">
      <header
        className="pointer-events-auto flex items-center gap-4 rounded-full border border-border px-4 py-2 transition-all duration-300"
        style={{
          background: scrolled
            ? "color-mix(in srgb, var(--background) 85%, transparent)"
            : "var(--background)",
          backdropFilter: scrolled ? "blur(16px)" : "blur(0px)",
          boxShadow: scrolled
            ? "0 4px 24px oklch(0 0 0 / 0.18)"
            : "0 1px 8px oklch(0 0 0 / 0.08)",
        }}
      >
        {/* Logo */}
        <Link href="/" className="flex items-center gap-1.5 group shrink-0">
          <span className="h-3.5 w-[3px] rounded-full bg-primary" />
          <span className="font-mono font-bold text-sm tracking-[-0.05em] group-hover:text-primary transition-colors duration-200">
            TEMPLAR
          </span>
        </Link>

        {/* Divider */}
        <span className="h-4 w-px bg-border shrink-0" />

        {/* Nav */}
        <NavHeader />

        {/* Divider */}
        <span className="h-4 w-px bg-border shrink-0" />

        {/* Controls */}
        <div className="flex items-center gap-2 shrink-0">
          <ThemeToggle />
          <UserButton />
        </div>
      </header>
    </div>
  );
}
