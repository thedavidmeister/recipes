import adapter from "@sveltejs/adapter-static";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [
    tailwindcss(),
    sveltekit({
      compilerOptions: {
        // Force runes mode for the project, except for libraries.
        runes: ({ filename }) =>
          filename.split(/[/\\]/).includes("node_modules") ? undefined : true,
      },
      // Static single-page app: client-rendered (see +layout.ts), so a
      // fallback shell serves every route. Host-agnostic (Vercel / CF Pages).
      adapter: adapter({ fallback: "index.html" }),
    }),
  ],
});
