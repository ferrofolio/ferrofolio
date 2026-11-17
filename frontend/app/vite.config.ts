import { rmSync, readFileSync } from "node:fs"
import { defineConfig, loadEnv } from "vite"
import react from "@vitejs/plugin-react"
import { sentryVitePlugin } from "@sentry/vite-plugin"
import { resolve } from "node:path"
import { createRequire } from "node:module"
import pkg from "./package.json"

const require = createRequire(import.meta.url)

export default defineConfig(({ command, mode }) => {
  const env = loadEnv(mode, __dirname, "")
  const isMobileDev = mode === "mobile" || process.env.MOBILE_DEV === "1"
  const isMobileBuild = process.env.MOBILE_BUILD === "1"
  const isMobile = isMobileDev || isMobileBuild
  const includeConnections = process.env.INCLUDE_CONNECTIONS !== "0"
  if (!isMobile) {
    rmSync("dist-electron", { recursive: true, force: true })
  }

  const isServe = command === "serve"
  const isBuild = command === "build"
  const sourcemap = isServe || !!process.env.VSCODE_DEBUG

  const serverFromUrl = (
    urlString: string | undefined,
    fallback: { host: string; port: number } = {
      host: "localhost",
      port: 5173,
    },
    strictPort: boolean = false,
  ) => {
    try {
      if (!urlString) return { ...fallback, strictPort }
      const url = new URL(urlString)
      return {
        host: url.hostname || fallback.host,
        port: url.port ? +url.port : fallback.port,
        strictPort,
      }
    } catch {
      return { ...fallback, strictPort }
    }
  }

  const mobileServer =
    isMobileDev && isServe ? serverFromUrl(env.VITE_DEV_SERVER_URL) : undefined

  const vscodeServer = process.env.VSCODE_DEBUG
    ? serverFromUrl((pkg as any)?.debug?.env?.VITE_DEV_SERVER_URL)
    : undefined

  const electronExternals = ["electron"]

  const electronPlugin = !isMobile
    ? (require("vite-plugin-electron/simple") as { default: any }).default
    : null

  // npm_package_version is unset under `pnpm exec`, which package.py uses.
  const appVersion: string = JSON.parse(
    readFileSync(resolve(__dirname, "package.json"), "utf-8"),
  ).version

  // Stack traces reaching Better Stack are minified unless the maps are uploaded.
  const sourceMapAuthToken = env.BETTER_STACK_API_TOKEN
  const uploadSourceMaps = isBuild && !!sourceMapAuthToken

  if (isBuild) {
    console.log(
      uploadSourceMaps
        ? `[sourcemaps] uploading to Better Stack (release ${appVersion})`
        : "[sourcemaps] upload skipped, BETTER_STACK_API_TOKEN is not set",
    )
  }

  return {
    define: {
      __MOBILE__: JSON.stringify(isMobile),
      __CONNECTIONS__: JSON.stringify(includeConnections),
      __APP_VERSION__: JSON.stringify(appVersion),
    },
    resolve: {
      alias: {
        "@": resolve(__dirname, "./src"),
      },
    },
    plugins: [
      react(),
      ...(!isMobileDev && electronPlugin
        ? [
            electronPlugin({
              main: {
                entry: "electron/main/index.ts",
                onstart: async ({ startup }) => {
                  await startup(["."])
                },
                vite: {
                  define: {
                    __APP_VERSION__: JSON.stringify(appVersion),
                    __BS_ELECTRON_MAIN_DSN__: JSON.stringify(
                      env.VITE_BS_ELECTRON_MAIN_DSN ?? "",
                    ),
                    __BS_DESKTOP_BACKEND_DSN__: JSON.stringify(
                      env.VITE_BS_DESKTOP_BACKEND_DSN ?? "",
                    ),
                    __BS_ENVIRONMENT__: JSON.stringify(
                      env.VITE_BS_ENVIRONMENT ||
                        (isBuild ? "production" : "development"),
                    ),
                  },
                  build: {
                    sourcemap: uploadSourceMaps ? "hidden" : sourcemap,
                    minify: isBuild,
                    outDir: "dist-electron/main",
                    rollupOptions: {
                      external: electronExternals,
                    },
                  },
                  plugins: uploadSourceMaps
                    ? [
                        sentryVitePlugin({
                          org: env.BETTER_STACK_SOURCEMAP_ORG,
                          project: env.BETTER_STACK_SOURCEMAP_PROJECT,
                          url: env.BETTER_STACK_SOURCEMAP_URL,
                          authToken: sourceMapAuthToken,
                          release: { name: appVersion },
                          sourcemaps: {
                            filesToDeleteAfterUpload: [
                              "dist-electron/main/**/*.map",
                            ],
                          },
                        }),
                      ]
                    : [],
                },
              },
              preload: {
                input: "electron/preload/index.ts",
                vite: {
                  build: {
                    sourcemap: sourcemap ? "inline" : undefined,
                    minify: isBuild,
                    outDir: "dist-electron/preload",
                    rollupOptions: {
                      external: electronExternals,
                      output: {
                        inlineDynamicImports: true,
                      },
                    },
                  },
                },
              },
            }),
          ]
        : []),
      ...(uploadSourceMaps
        ? [
            sentryVitePlugin({
              org: env.BETTER_STACK_SOURCEMAP_ORG,
              project: env.BETTER_STACK_SOURCEMAP_PROJECT,
              url: env.BETTER_STACK_SOURCEMAP_URL,
              authToken: sourceMapAuthToken,
              release: { name: appVersion },
              sourcemaps: {
                filesToDeleteAfterUpload: ["dist/**/*.map"],
              },
            }),
          ]
        : []),
    ],
    build: {
      sourcemap: uploadSourceMaps ? "hidden" : sourcemap,
      rollupOptions: {
        input: {
          main: resolve(__dirname, "index.html"),
          about: resolve(__dirname, "about.html"),
        },
        external: isMobile
          ? [
              "electron",
              "electron-updater",
              "electron-window-state",
              "electron-is-dev",
            ]
          : [],
      },
    },
    server: mobileServer || vscodeServer,
  }
})
