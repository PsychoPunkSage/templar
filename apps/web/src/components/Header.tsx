"use client";

// Shared nav bar — rendered globally via app/layout.tsx.
// Client component because ThemeToggle uses hooks.

import Link from "next/link";
import { usePathname } from "next/navigation";
import { ThemeToggle } from "@/components/ThemeToggle";

function NavLink({
  href,
  children,
}: {
  href: string;
  children: React.ReactNode;
}) {
  const pathname = usePathname();
  // Highlight the link if we're on that path (or a sub-path of it)
  const isActive = pathname === href || pathname.startsWith(href + "/");

  return (
    <Link
      href={href}
      className={`text-sm font-medium transition-colors hover:text-foreground ${
        isActive ? "text-foreground" : "text-muted-foreground"
      }`}
    >
      {children}
    </Link>
  );
}

export function Header() {
  return (
    <header className="sticky top-0 z-50 flex items-center justify-between px-6 py-3 border-b bg-background/95 backdrop-blur shrink-0">
      {/* Logo */}
      <div className="flex items-center gap-6">
        <Link href="/" className="flex items-center gap-2">
          <span className="font-bold text-lg tracking-tight">Templar</span>
          <span className="text-xs text-muted-foreground hidden sm:block">
            AI Resume Engine
          </span>
        </Link>

        {/* Primary navigation */}
        <nav className="flex items-center gap-4">
          <NavLink href="/projects">Projects</NavLink>
          <NavLink href="/context">Context</NavLink>
        </nav>
      </div>

      {/* Right side controls */}
      <ThemeToggle />
    </header>
  );
}
