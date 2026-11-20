import { readFileSync } from "node:fs"
import { defineConfig, loadEnv } from "vite"
import react from "@vitejs/plugin-react"
import { sentryVitePlugin } from "@sentry/vite-plugin"
import { resolve } from "node:path"
import { serviceWorkerPlugin } from "./pwa/serviceWorkerPlugin"

export default defineConfig(({ command, mode }) => {
  const env = loadEnv(mode, __dirname, "")

  const isServe = command === "serve"
  const isBuild = command === "build"

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
      __APP_VERSION__: JSON.stringify(appVersion),
    },
    resolve: {
      alias: {
        "@": resolve(__dirname, "./src"),
      },
    },
    plugins: [
      react(),
      serviceWorkerPlugin(),
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
      sourcemap: uploadSourceMaps ? "hidden" : isServe,
    },
  }
})
