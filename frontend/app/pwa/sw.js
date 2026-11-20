// Service worker of the web app. pwa/serviceWorkerPlugin.ts emits it at build time and fills in
// the paths of the app shell and a version that changes with every build.
//
// It caches the app shell only: the HTML, scripts, styles and icons of the build. It never stores
// API responses or anything else that is not in the shell list, so no financial data lands in
// the browser's caches. A new version waits until the user accepts the reload prompt.
//
// SHELL.precache is what the first screen needs and is fetched on install. The other files of
// the shell, chunks that load on demand, are cached the first time they are used.

const SHELL = /* shell */ { precache: [], onDemand: [] }
const CACHE_PREFIX = "ferrofolio-shell-"
const CACHE_NAME = CACHE_PREFIX + /* version */ "dev"
const SHELL_PATHS = new Set([...SHELL.precache, ...SHELL.onDemand])
const INDEX_PATHS = new Set(["/", "/index.html"])

self.addEventListener("install", event => {
  event.waitUntil(
    caches
      .open(CACHE_NAME)
      .then(cache =>
        cache.addAll(
          SHELL.precache.map(path => new Request(path, { cache: "reload" })),
        ),
      ),
  )
})

self.addEventListener("activate", event => {
  event.waitUntil(
    caches
      .keys()
      .then(names =>
        Promise.all(
          names
            .filter(
              name => name.startsWith(CACHE_PREFIX) && name !== CACHE_NAME,
            )
            .map(name => caches.delete(name)),
        ),
      ),
  )
})

self.addEventListener("message", event => {
  if (event.data && event.data.type === "SKIP_WAITING") {
    self.skipWaiting()
  }
})

function shellKey(request) {
  if (request.method !== "GET") return null
  const url = new URL(request.url)
  if (url.origin !== self.location.origin) return null
  if (request.mode === "navigate") {
    return INDEX_PATHS.has(url.pathname) ? "/" : null
  }
  return SHELL_PATHS.has(url.pathname) ? url.pathname : null
}

async function fromShell(key, request) {
  const cache = await caches.open(CACHE_NAME)
  const cached = await cache.match(key)
  if (cached) return cached
  const response = await fetch(request)
  if (response.ok && response.type === "basic") {
    await cache.put(key, response.clone())
  }
  return response
}

self.addEventListener("fetch", event => {
  const key = shellKey(event.request)
  if (key === null) return
  event.respondWith(fromShell(key, event.request))
})
