import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  output: "standalone",
  // Turbopack configuration (Next.js 16+ default bundler)
  // PDF.js canvas alias: the `canvas` npm package is not available in browsers;
  // we resolve it to false to prevent bundler errors.
  turbopack: {
    resolveAlias: {
      canvas: { browser: "./src/lib/empty-module.ts" },
    },
  },
};

export default nextConfig;
