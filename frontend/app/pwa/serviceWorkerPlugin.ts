import { createHash } from "node:crypto"
import { readFileSync } from "node:fs"
import { resolve } from "node:path"
import type { Plugin } from "vite"

// Files of public/ that belong to the app shell: the manifest, the icons that index.html and the
// manifest reference, and the logo of the splash and login screens. Institution logos are not
// part of it; they are many and load on demand.
export const PUBLIC_SHELL_FILES = [
  "site.webmanifest",
  "favicon.ico",
  "favicon-16x16.png",
  "favicon-32x32.png",
  "apple-touch-icon.png",
  "android-chrome-192x192.png",
  "android-chrome-512x512.png",
  "ferrofolio-fg.svg",
]

export const SERVICE_WORKER_FILE = "sw.js"

const SHELL_PLACEHOLDER = "/* shell */ { precache: [], onDemand: [] }"
const VERSION_PLACEHOLDER = '/* version */ "dev"'

// The parts of a Rollup output bundle the shell is computed from.
export type BundleFile =
  | {
      type: "chunk"
      isEntry: boolean
      imports: string[]
      viteMetadata?: {
        importedCss: Set<string>
        importedAssets: Set<string>
      }
    }
  | { type: "asset" }

export interface Shell {
  // Fetched when the worker installs: index.html as "/", the entry chunks with their static
  // imports, styles and assets, and the public shell files.
  precache: string[]
  // Every other file of the build, cached when it is first loaded.
  onDemand: string[]
}

function isShellFile(file: string): boolean {
  return (
    file !== "index.html" &&
    file !== SERVICE_WORKER_FILE &&
    !file.endsWith(".map")
  )
}

export function computeShell(
  bundle: Record<string, BundleFile>,
  publicFiles: string[] = PUBLIC_SHELL_FILES,
): Shell {
  const initial = new Set<string>()
  const visit = (file: string) => {
    const entry = bundle[file]
    if (initial.has(file) || !entry) return
    initial.add(file)
    if (entry.type !== "chunk") return
    entry.viteMetadata?.importedCss.forEach(css => initial.add(css))
    entry.viteMetadata?.importedAssets.forEach(asset => initial.add(asset))
    entry.imports.forEach(visit)
  }
  Object.entries(bundle).forEach(([file, entry]) => {
    if (entry.type === "chunk" && entry.isEntry) visit(file)
  })

  const toPath = (file: string) => `/${file}`
  const files = Object.keys(bundle).filter(isShellFile)
  return {
    precache: [
      "/",
      ...[...files.filter(file => initial.has(file)), ...publicFiles]
        .sort()
        .map(toPath),
    ],
    onDemand: files
      .filter(file => !initial.has(file))
      .sort()
      .map(toPath),
  }
}

// Hashes the shell, index.html and the contents of the public shell files. Bundle files carry a
// content hash in their names; public files do not, so their contents count here.
export function shellVersion(
  shell: Shell,
  indexHtml: string,
  publicContents: Uint8Array[] = [],
): string {
  const hash = createHash("sha256")
    .update(JSON.stringify(shell))
    .update(indexHtml)
  publicContents.forEach(content => hash.update(content))
  return hash.digest("hex").slice(0, 16)
}

export function renderServiceWorker(
  template: string,
  shell: Shell,
  version: string,
): string {
  if (
    !template.includes(SHELL_PLACEHOLDER) ||
    !template.includes(VERSION_PLACEHOLDER)
  ) {
    throw new Error("the service worker template lacks its placeholders")
  }
  return template
    .replace(SHELL_PLACEHOLDER, () => JSON.stringify(shell))
    .replace(VERSION_PLACEHOLDER, () => JSON.stringify(version))
}

// Emits sw.js into the build, with the shell of this build and a version that changes with it.
export function serviceWorkerPlugin(): Plugin {
  return {
    name: "ferrofolio:service-worker",
    apply: "build",
    enforce: "post",
    generateBundle(_options, bundle) {
      const index = bundle["index.html"]
      if (!index || index.type !== "asset") {
        this.error("index.html is missing from the bundle")
      }
      const shell = computeShell(bundle as Record<string, BundleFile>)
      const template = readFileSync(resolve(__dirname, "sw.js"), "utf-8")
      const publicContents = PUBLIC_SHELL_FILES.map(file =>
        readFileSync(resolve(__dirname, "../public", file)),
      )
      this.emitFile({
        type: "asset",
        fileName: SERVICE_WORKER_FILE,
        source: renderServiceWorker(
          template,
          shell,
          shellVersion(shell, String(index.source), publicContents),
        ),
      })
    },
  }
}
