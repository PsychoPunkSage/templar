"use client";

import { useRef, useState, useCallback } from "react";
import { cn } from "@/lib/utils";

interface Circle {
  id: number;
  x: number;
  y: number;
}

interface HoverButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  children: React.ReactNode;
  variant?: "primary" | "ghost";
}

let _cid = 0;

export function HoverButton({
  children,
  className,
  variant = "primary",
  disabled,
  ...props
}: HoverButtonProps) {
  const btnRef = useRef<HTMLButtonElement>(null);
  const [circles, setCircles] = useState<Circle[]>([]);
  const [hovered, setHovered] = useState(false);
  const lastPos = useRef({ x: -999, y: -999 });
  const isPrimary = variant === "primary";

  const handleMouseMove = useCallback(
    (e: React.MouseEvent<HTMLButtonElement>) => {
      if (disabled) return;
      const rect = btnRef.current?.getBoundingClientRect();
      if (!rect) return;
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;
      const dx = x - lastPos.current.x;
      const dy = y - lastPos.current.y;
      if (dx * dx + dy * dy < 64) return;
      lastPos.current = { x, y };
      const id = ++_cid;
      setCircles((prev) => [...prev, { id, x, y }]);
      setTimeout(() => setCircles((prev) => prev.filter((c) => c.id !== id)), 550);
    },
    [disabled]
  );

  return (
    <button
      ref={btnRef}
      onMouseMove={handleMouseMove}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => {
        setHovered(false);
        lastPos.current = { x: -999, y: -999 };
      }}
      disabled={disabled}
      className={cn(
        "relative overflow-hidden rounded-md px-5 py-2.5 text-sm font-semibold",
        "select-none inline-flex items-center gap-2",
        isPrimary
          ? "bg-primary text-primary-foreground"
          : "bg-transparent text-foreground border border-border",
        "disabled:opacity-40 disabled:cursor-not-allowed",
        className
      )}
      style={{
        boxShadow:
          hovered && !disabled
            ? isPrimary
              ? "0 0 28px oklch(0.62 0.22 255 / 45%), 0 0 8px oklch(0.62 0.22 255 / 25%)"
              : "0 0 18px oklch(0.62 0.22 255 / 22%)"
            : "none",
        transform: hovered && !disabled ? "translateY(-1px)" : "none",
        transition: "box-shadow 0.25s ease, transform 0.15s ease",
      }}
      {...props}
    >
      {circles.map((c) => (
        <span
          key={c.id}
          className="pointer-events-none absolute rounded-full"
          style={{
            width: 20,
            height: 20,
            left: c.x - 10,
            top: c.y - 10,
            background: isPrimary
              ? "oklch(0.88 0.12 255 / 0.65)"
              : "oklch(0.62 0.22 255 / 0.45)",
            animation: "hb-circle-trail 0.55s ease-out forwards",
          }}
        />
      ))}

      <span className="relative z-10 flex items-center gap-2">{children}</span>

      <style>{`
        @keyframes hb-circle-trail {
          0%   { opacity: 0.8; transform: scale(0.6); }
          40%  { opacity: 0.5; transform: scale(1.1); }
          100% { opacity: 0;   transform: scale(2.2); }
        }
      `}</style>
    </button>
  );
}
