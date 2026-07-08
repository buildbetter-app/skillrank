import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

/** @type {import('next').NextConfig} */
const nextConfig = {
  // NOT static-export: we need middleware to serve the install script to curl.
  trailingSlash: true,
  turbopack: {
    root: __dirname
  }
};

export default nextConfig;
