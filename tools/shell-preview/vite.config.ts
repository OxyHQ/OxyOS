import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const root = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const extensions = [
  ".web.tsx",
  ".web.ts",
  ".web.jsx",
  ".web.js",
  ".tsx",
  ".ts",
  ".jsx",
  ".js",
  ".json",
];
const soundModule = /^.*\/fabric\/NativeSvgSoundModule$/;
const soundWeb = path.join(root, "src/android-sound-module.web.ts");

export default defineConfig(({ mode }) => ({
  plugins: [
    react(),
    tailwindcss(),
    {
      name: "oxyos-source-dependencies",
      enforce: "pre",
      async resolveId(source, importer) {
        // Shared OxyOS source consumes the development host's dependencies.
        if (
          importer?.includes("/apps/shell/") &&
          !source.startsWith(".") &&
          !path.isAbsolute(source)
        )
          return this.resolve(source, path.join(root, "src/main.tsx"), {
            skipSelf: true,
          });
      },
    },
    {
      name: "shell-source-refresh",
      configureServer(server) {
        const source = path.resolve(root, "../../apps/shell/src");
        server.watcher.add(source);
        const refresh = (file: string) => {
          if (file.startsWith(source + path.sep))
            server.ws.send({ type: "full-reload", path: "*" });
        };
        server.watcher.on("change", refresh);
        server.httpServer?.once("close", () =>
          server.watcher.off("change", refresh),
        );
      },
    },
    {
      name: "expo-web-bootstrap",
      transform(code, id) {
        if (
          /\/expo-modules-core\/src\/ensureNativeModulesAreInstalled\.ts$/.test(
            id.split("?")[0],
          )
        )
          return { code, map: null, moduleSideEffects: true };
      },
    },
  ],
  resolve: {
    alias: [
      { find: /^react-native$/, replacement: "react-native-web" },
      {
        find: "react-native/Libraries/Image/AssetRegistry",
        replacement: "react-native-web/dist/modules/AssetRegistry",
      },
      {
        find: "@react-native/assets-registry/registry",
        replacement: "react-native-web/dist/modules/AssetRegistry",
      },
      {
        find: /^expo-modules-core$/,
        replacement: path.join(
          path.dirname(require.resolve("expo/package.json")),
          "../expo-modules-core/src/index.ts",
        ),
      },
      { find: soundModule, replacement: soundWeb },
    ],
    extensions,
    dedupe: ["react", "react-dom", "react-native-web"],
  },
  define: {
    __DEV__: JSON.stringify(mode !== "production"),
    global: "globalThis",
    "process.env.EXPO_OS": '"web"',
    "process.env.JEST_WORKER_ID": "undefined",
    "process.env": "{}",
  },
  server: { fs: { allow: [path.resolve(root, "../..")] } },
  optimizeDeps: {
    include: ["react-native-web"],
    exclude: ["react-native"],
    esbuildOptions: {
      loader: { ".js": "jsx" },
      resolveExtensions: extensions,
      plugins: [
        {
          name: "svg-sound-web",
          setup(build) {
            build.onResolve({ filter: soundModule }, () => ({
              path: soundWeb,
            }));
          },
        },
      ],
    },
  },
  build: {
    commonjsOptions: {
      transformMixedEsModules: true,
      ignoreTryCatch: (id) => !id.startsWith("react-native-web/dist/"),
    },
  },
}));
