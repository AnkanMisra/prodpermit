import type { NextConfig } from "next";

const backendUrl = process.env.BACKEND_URL ?? "http://127.0.0.1:8080";

const nextConfig = {
  poweredByHeader: false,
  allowedDevOrigins: [
    "ankan-linux.tailf04855.ts.net",
    "100.121.232.117",
    "192.168.31.133"
  ],
  async headers() {
    return [
      {
        source: "/:path*",
        headers: [
          { key: "Origin-Agent-Cluster", value: "?1" },
          { key: "Permissions-Policy", value: "tools=(self)" },
          { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
          { key: "X-Content-Type-Options", value: "nosniff" }
        ]
      }
    ];
  },
  async rewrites() {
    return [
      {
        source: "/api/backend/:path*",
        destination: `${backendUrl}/api/:path*`
      }
    ];
  }
} satisfies NextConfig;

export default nextConfig;

