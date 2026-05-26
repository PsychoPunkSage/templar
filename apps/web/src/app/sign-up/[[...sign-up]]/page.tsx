"use client";

import { SignUp } from "@clerk/nextjs";
import { ShaderBackground } from "@/components/ui/shader-background";

export default function SignUpPage() {
  return (
    <div className="relative flex min-h-screen items-center justify-center px-4">
      <ShaderBackground className="fixed inset-0 -z-10" intensity={0.9} />

      {/* Glass card */}
      <div className="w-full max-w-md rounded-2xl border border-white/10 bg-black/50 backdrop-blur-xl shadow-2xl p-8 flex flex-col items-center gap-6">
        {/* Branding */}
        <div className="flex flex-col items-center gap-2 text-center">
          <div className="flex items-center gap-2">
            <span className="h-4 w-[3px] rounded-full bg-primary" />
            <span className="font-mono font-bold text-lg tracking-[-0.05em] text-white">
              TEMPLAR
            </span>
          </div>
          <p className="text-xs font-mono tracking-[0.15em] uppercase text-white/40">
            AI Resume Engine
          </p>
          <p className="text-xs text-white/50 mt-1">
            Every bullet grounded. Every line verified.
          </p>
        </div>

        {/* Clerk widget */}
        <SignUp />
      </div>
    </div>
  );
}
