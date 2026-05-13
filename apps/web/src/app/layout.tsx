import type { Metadata } from "next";
import { Inter } from "next/font/google";
import { ThemeProvider } from "next-themes";
import { ClerkProvider } from "@clerk/nextjs";
import { Header } from "@/components/Header";
import { AuthSync } from "@/components/AuthSync";
import { PHProvider } from "@/components/PHProvider";
import "./globals.css";

const inter = Inter({ subsets: ["latin"] });

export const metadata: Metadata = {
  title: "Templar — AI Resume Engine",
  description:
    "Context-aware, layout-optimized resume generation. Every bullet grounded, every line verified.",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <ClerkProvider>
      <html lang="en" suppressHydrationWarning>
        <body className={inter.className}>
          <PHProvider>
            <ThemeProvider attribute="class" defaultTheme="system" enableSystem>
              <AuthSync />
              {/* Header is sticky and rendered globally — individual pages do NOT
                  render their own header anymore. The editor page still has an
                  action bar (project name + Generate button), but not the nav. */}
              <Header />
              <main className="flex flex-col min-h-[calc(100vh-53px)]">
                {children}
              </main>
            </ThemeProvider>
          </PHProvider>
        </body>
      </html>
    </ClerkProvider>
  );
}
