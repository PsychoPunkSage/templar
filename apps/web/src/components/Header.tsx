"use client";

// Shared nav bar — rendered globally via app/layout.tsx.
// Client component because ThemeToggle uses hooks.

import Link from "next/link";
import { usePathname } from "next/navigation";
import { UserButton } from "@clerk/nextjs";
import { ThemeToggle } from "@/components/ThemeToggle";

function NavLink({
  href,
  children,
}: {
  href: string;
  children: React.ReactNode;
}) {
  const pathname = usePathname();
  const isActive = pathname === href || pathname.startsWith(href + "/");

  return (
    <Link
      href={href}
      className={`text-sm font-medium px-3 py-1.5 rounded-md transition-colors ${
        isActive
          ? "bg-accent text-accent-foreground"
          : "text-muted-foreground hover:text-foreground hover:bg-accent/50"
      }`}
    >
      {children}
    </Link>
  );
}

export function Header() {
  return (
    <header className="sticky top-0 z-50 flex items-center justify-between px-6 py-2.5 border-b bg-background/80 backdrop-blur-md shrink-0">
      {/* Logo */}
      <div className="flex items-center gap-4">
        <Link href="/" className="flex items-center gap-2 mr-2">
          <span className="font-bold text-lg tracking-tight">Templar</span>
          <span className="text-xs text-muted-foreground hidden sm:block">
            AI Resume Engine
          </span>
        </Link>

        {/* Primary navigation */}
        <nav className="flex items-center gap-1">
          <NavLink href="/projects">Projects</NavLink>
          <NavLink href="/context">Context</NavLink>
          <NavLink href="/personas">Personas</NavLink>
          <NavLink href="/profile">Profile</NavLink>
        </nav>
      </div>

      {/* Right side controls */}
      <div className="flex items-center gap-3">
        <ThemeToggle />
        <UserButton />
      </div>
    </header>
  );
}
