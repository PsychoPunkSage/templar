"use client";

import { useRef, useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { motion } from "framer-motion";

const NAV_ITEMS = [
  { href: "/projects", label: "Projects" },
  { href: "/context", label: "Context" },
  { href: "/personas", label: "Personas" },
  { href: "/profile", label: "Profile" },
];

function Tab({
  href,
  children,
  setPosition,
}: {
  href: string;
  children: React.ReactNode;
  setPosition: React.Dispatch<
    React.SetStateAction<{ left: number; width: number; opacity: number }>
  >;
}) {
  const ref = useRef<HTMLLIElement>(null);
  const pathname = usePathname();
  const isActive = pathname === href || pathname.startsWith(href + "/");

  return (
    <li
      ref={ref}
      onMouseEnter={() => {
        if (!ref.current) return;
        const { width } = ref.current.getBoundingClientRect();
        setPosition({ width, opacity: 1, left: ref.current.offsetLeft });
      }}
      className="relative z-10 block cursor-pointer"
    >
      <Link
        href={href}
        className={`block px-3 py-1.5 text-xs font-mono tracking-widest uppercase mix-blend-difference ${
          isActive ? "text-white font-semibold" : "text-white"
        }`}
      >
        {children}
      </Link>
    </li>
  );
}

function Cursor({
  position,
}: {
  position: { left: number; width: number; opacity: number };
}) {
  return (
    <motion.li
      animate={position}
      className="absolute z-0 h-7 rounded-full bg-primary"
      style={{ top: "50%", transform: "translateY(-50%)" }}
    />
  );
}

export function NavHeader() {
  const [position, setPosition] = useState({ left: 0, width: 0, opacity: 0 });

  return (
    <ul
      className="relative flex w-fit items-center rounded-full border border-border bg-muted/40 p-1"
      onMouseLeave={() => setPosition((pv) => ({ ...pv, opacity: 0 }))}
    >
      {NAV_ITEMS.map((item) => (
        <Tab key={item.href} href={item.href} setPosition={setPosition}>
          {item.label}
        </Tab>
      ))}
      <Cursor position={position} />
    </ul>
  );
}
