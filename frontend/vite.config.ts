import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In development the API runs separately (cargo run in ../server); in the container nginx
// does this proxying instead.
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: { "/api": process.env.API_URL ?? "http://localhost:8080" },
  },
});
