import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  base: "/demo/",
  build: {
    outDir: "web-build",
    target: "esnext",
    rollupOptions: {
      input: "web.html",
    },
  },
});
