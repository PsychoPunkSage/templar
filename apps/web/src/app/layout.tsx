import type { Metadata } from "next";
import { Inter, IBM_Plex_Mono } from "next/font/google";
import { ThemeProvider } from "next-themes";
import { ClerkProvider } from "@clerk/nextjs";
import { AuthSync } from "@/components/AuthSync";
import { PHProvider } from "@/components/PHProvider";
import "./globals.css";

const inter = Inter({ subsets: ["latin"], variable: "--font-sans" });
const ibmPlexMono = IBM_Plex_Mono({
  subsets: ["latin"],
  weight: ["400", "500", "600", "700"],
  variable: "--font-mono",
});

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
        <body className={`${inter.variable} ${ibmPlexMono.variable} font-sans`}>
          <PHProvider>
            <ThemeProvider attribute="class" defaultTheme="dark" enableSystem>
              <AuthSync />
              {children}
            </ThemeProvider>
          </PHProvider>
        </body>
      </html>
    </ClerkProvider>
  );
}
