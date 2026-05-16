"use client";

import { useRef, useState, useCallback } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const md3Variants = cva(
  [
    "relative overflow-hidden text-sm font-medium select-none",
    "inline-flex items-center justify-center gap-2 cursor-pointer",
    "transition-colors duration-150",
    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1",
    "disabled:opacity-40 disabled:cursor-not-allowed",
  ].join(" "),
  {
    variants: {
      variant: {
        filled:   "bg-primary text-primary-foreground hover:bg-primary/90",
        tonal:    "bg-accent text-accent-foreground hover:bg-accent/80",
        outlined: "border border-border bg-transparent text-foreground hover:bg-muted/50",
        text:     "bg-transparent text-primary hover:bg-primary/10",
        /* extended aliases used by some editor UI */
        default:  "bg-primary text-primary-foreground hover:bg-primary/90",
        secondary:"bg-accent text-accent-foreground hover:bg-accent/80",
        outline:  "border border-border bg-transparent text-foreground hover:bg-muted/50",
        ghost:    "bg-transparent text-primary hover:bg-primary/10",
      },
      size: {
        sm:  "px-3 py-1.5 text-xs rounded-md",
        md:  "px-4 py-2 rounded-md",
        lg:  "px-6 py-2.5 rounded-lg",
        xl:  "px-8 py-3 text-base rounded-xl",
        "2xl": "px-10 py-4 text-base rounded-2xl",
        icon:    "h-9 w-9 rounded-md",
        "icon-sm": "h-7 w-7 rounded-md",
        "icon-lg": "h-11 w-11 rounded-lg",
      },
    },
    defaultVariants: {
      variant: "filled",
      size: "md",
    },
  }
);

type RippleState = "expanding" | "fading";

interface Ripple {
  id: number;
  x: number;
  y: number;
  size: number;
  state: RippleState;
}

let rippleId = 0;

interface Md3ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof md3Variants> {
  shape?: "round" | "square";
}

export function Md3Button({
  children,
  className,
  variant,
  size,
  shape = "square",
  disabled,
  onMouseDown,
  onMouseUp,
  onMouseLeave,
  ...props
}: Md3ButtonProps) {
  const btnRef = useRef<HTMLButtonElement>(null);
  const [ripples, setRipples] = useState<Ripple[]>([]);
  const [pressed, setPressed] = useState(false);

  const addRipple = useCallback(
    (e: React.MouseEvent<HTMLButtonElement>) => {
      if (disabled) return;
      const rect = btnRef.current!.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;
      const size = Math.max(rect.width, rect.height) * 2.2;
      const id = rippleId++;
      setRipples((prev) => [...prev, { id, x, y, size, state: "expanding" }]);
      setTimeout(() => {
        setRipples((prev) =>
          prev.map((r) => (r.id === id ? { ...r, state: "fading" } : r))
        );
        setTimeout(() => {
          setRipples((prev) => prev.filter((r) => r.id !== id));
        }, 380);
      }, 280);
    },
    [disabled]
  );

  const isRound =
    variant === "filled" || variant === "default" ||
    variant === "tonal" || variant === "secondary" ||
    shape === "round";

  return (
    <button
      ref={btnRef}
      disabled={disabled}
      onMouseDown={(e) => {
        setPressed(true);
        addRipple(e);
        onMouseDown?.(e);
      }}
      onMouseUp={(e) => {
        setPressed(false);
        onMouseUp?.(e);
      }}
      onMouseLeave={(e) => {
        setPressed(false);
        onMouseLeave?.(e);
      }}
      className={cn(md3Variants({ variant, size }), className)}
      style={{
        borderRadius: pressed
          ? isRound
            ? "999px"
            : "4px"
          : undefined,
        transition: "border-radius 0.14s ease, background-color 0.15s ease",
      }}
      {...props}
    >
      {ripples.map((r) => (
        <span
          key={r.id}
          className="pointer-events-none absolute rounded-full"
          style={{
            width: r.size,
            height: r.size,
            left: r.x - r.size / 2,
            top: r.y - r.size / 2,
            background: "currentColor",
            opacity: r.state === "expanding" ? 0.12 : 0,
            transform: r.state === "expanding" ? "scale(1)" : "scale(1.15)",
            transition:
              r.state === "expanding"
                ? "transform 0.38s ease-out, opacity 0.38s ease-out"
                : "opacity 0.38s ease-out, transform 0.38s ease-out",
          }}
        />
      ))}
      <span className="relative z-10 flex items-center gap-2">{children}</span>
    </button>
  );
}

// ── SplitButton ────────────────────────────────────────────────────────────

interface SplitButtonProps {
  primary: React.ReactNode;
  onPrimaryClick?: () => void;
  secondary: React.ReactNode;
  onSecondaryClick?: () => void;
  variant?: Md3ButtonProps["variant"];
  size?: Md3ButtonProps["size"];
  disabled?: boolean;
  className?: string;
}

export function SplitButton({
  primary,
  onPrimaryClick,
  secondary,
  onSecondaryClick,
  variant = "filled",
  size = "md",
  disabled,
  className,
}: SplitButtonProps) {
  return (
    <div className={cn("inline-flex rounded-md overflow-hidden", className)}>
      <Md3Button
        variant={variant}
        size={size}
        disabled={disabled}
        onClick={onPrimaryClick}
        className="rounded-r-none border-r border-r-white/20"
      >
        {primary}
      </Md3Button>
      <Md3Button
        variant={variant}
        size={size}
        disabled={disabled}
        onClick={onSecondaryClick}
        className="rounded-l-none px-2"
      >
        {secondary}
      </Md3Button>
    </div>
  );
}
