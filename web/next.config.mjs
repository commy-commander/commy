/** @type {import('next').NextConfig} */
const nextConfig = {
  // Standalone output keeps the Docker runtime image small
  output: "standalone",
};

export default nextConfig;
