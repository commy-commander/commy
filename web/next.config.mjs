/** @type {import('next').NextConfig} */
const nextConfig = {
  // Standalone output keeps the Docker runtime image small
  output: "standalone",
  async redirects() {
    return [{ source: "/docs/privacy", destination: "/privacy", permanent: true }];
  },
};

export default nextConfig;
