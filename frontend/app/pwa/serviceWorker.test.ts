import { readFileSync } from "node:fs"
import { resolve } from "node:path"
import { beforeEach, describe, expect, it, vi } from "vitest"
import {
  type BundleFile,
  PUBLIC_SHELL_FILES,
  computeShell,
  renderServiceWorker,
  shellVersion,
} from "./serviceWorkerPlugin"

const ORIGIN = "https://ferrofolio.example"
const TEMPLATE = readFileSync(resolve(__dirname, "sw.js"), "utf-8")
const SHELL = {
  precache: ["/", "/assets/index-abc.js", "/assets/index-abc.css"],
  onDemand: ["/assets/lazy-abc.js"],
}

type Handler = (event: any) => void

class FakeRequest {
  constructor(
    public url: string,
    public init: { cache?: string } = {},
  ) {}
}

function fakeResponse(body: string, ok = true, type = "basic") {
  return { body, ok, type, clone: () => fakeResponse(body, ok, type) }
}

function loadWorker(version = "v2", existingCaches: string[] = []) {
  const handlers: Record<string, Handler> = {}
  const stored = new Map<string, Map<string, unknown>>()
  let putFails = false
  existingCaches.forEach(name => stored.set(name, new Map()))
  const cacheFor = (name: string) => {
    if (!stored.has(name)) stored.set(name, new Map())
    const entries = stored.get(name)!
    return {
      addAll: async (requests: FakeRequest[]) => {
        requests.forEach(request => {
          expect(request.init.cache).toBe("reload")
          entries.set(request.url, `body of ${request.url}`)
        })
      },
      match: async (key: string) => entries.get(key),
      put: async (key: string, value: unknown) => {
        if (putFails) throw new Error("QuotaExceededError")
        entries.set(key, value)
      },
    }
  }
  const caches = {
    open: async (name: string) => cacheFor(name),
    keys: async () => [...stored.keys()],
    delete: async (name: string) => stored.delete(name),
  }
  const fetchMock = vi.fn(async () => fakeResponse("from network"))
  const self = {
    location: { origin: ORIGIN },
    skipWaiting: vi.fn(),
    addEventListener: (type: string, handler: Handler) => {
      handlers[type] = handler
    },
  }
  const source = renderServiceWorker(TEMPLATE, SHELL, version)
  new Function("self", "caches", "fetch", "Request", source)(
    self,
    caches,
    fetchMock,
    FakeRequest,
  )
  const cache = () => stored.get(`ferrofolio-shell-${version}`)!
  const failPut = () => {
    putFails = true
  }
  return { handlers, stored, cache, self, fetchMock, failPut }
}

async function runLifecycle(
  handler: Handler,
  event: Record<string, unknown> = {},
) {
  let pending: Promise<unknown> = Promise.resolve()
  handler({ ...event, waitUntil: (p: Promise<unknown>) => (pending = p) })
  await pending
}

async function dispatchFetch(
  handler: Handler,
  request: { url: string; method?: string; mode?: string },
) {
  let response: Promise<unknown> | undefined
  handler({
    request: { method: "GET", mode: "no-cors", ...request },
    respondWith: (p: Promise<unknown>) => (response = p),
  })
  return response === undefined ? undefined : await response
}

function chunk(
  imports: string[] = [],
  options: { isEntry?: boolean; css?: string[]; assets?: string[] } = {},
): BundleFile {
  return {
    type: "chunk",
    isEntry: options.isEntry ?? false,
    imports,
    viteMetadata: {
      importedCss: new Set(options.css ?? []),
      importedAssets: new Set(options.assets ?? []),
    },
  }
}

describe("computeShell", () => {
  const bundle: Record<string, BundleFile> = {
    "index.html": { type: "asset" },
    "assets/index.js": chunk(["assets/vendor.js"], {
      isEntry: true,
      css: ["assets/index.css"],
    }),
    "assets/vendor.js": chunk([], { assets: ["assets/font.woff2"] }),
    "assets/index.css": { type: "asset" },
    "assets/font.woff2": { type: "asset" },
    "assets/lazy.js": chunk(["assets/vendor.js"]),
    "assets/index.js.map": { type: "asset" },
    "sw.js": { type: "asset" },
  }

  it("precaches index.html as /, the entry with its imports, and the public files", () => {
    const shell = computeShell(bundle, ["site.webmanifest"])
    expect(shell.precache).toEqual([
      "/",
      "/assets/font.woff2",
      "/assets/index.css",
      "/assets/index.js",
      "/assets/vendor.js",
      "/site.webmanifest",
    ])
  })

  it("leaves lazy chunks to be cached on demand, without source maps or the worker", () => {
    expect(computeShell(bundle, []).onDemand).toEqual(["/assets/lazy.js"])
  })

  it("includes the manifest and its icons by default", () => {
    const shell = computeShell({ "index.html": { type: "asset" } })
    expect(shell.precache).toContain("/site.webmanifest")
    expect(shell.precache).toContain("/android-chrome-512x512.png")
    expect(shell.precache).toHaveLength(PUBLIC_SHELL_FILES.length + 1)
  })

  it("does not include the runtime configuration", () => {
    const shell = computeShell(bundle)
    expect([...shell.precache, ...shell.onDemand]).not.toContain(
      "/env-config.js",
    )
  })
})

describe("shellVersion", () => {
  it("changes with the shell and with index.html", () => {
    const shell = { precache: ["/", "/a.js"], onDemand: [] }
    const base = shellVersion(shell, "<html>")
    expect(shellVersion(shell, "<html>")).toBe(base)
    expect(
      shellVersion({ precache: ["/", "/b.js"], onDemand: [] }, "<html>"),
    ).not.toBe(base)
    expect(shellVersion(shell, "<html >")).not.toBe(base)
  })

  it("changes with the contents of the public shell files", () => {
    const shell = { precache: ["/", "/site.webmanifest"], onDemand: [] }
    const encode = (text: string) => new TextEncoder().encode(text)
    expect(shellVersion(shell, "<html>", [encode('{"name":"A"}')])).not.toBe(
      shellVersion(shell, "<html>", [encode('{"name":"B"}')]),
    )
  })
})

describe("renderServiceWorker", () => {
  it("fills in the shell and the version", () => {
    const source = renderServiceWorker(TEMPLATE, SHELL, "abc123")
    expect(source).toContain(`const SHELL = ${JSON.stringify(SHELL)}`)
    expect(source).toContain('CACHE_PREFIX + "abc123"')
  })

  it("fails on a template without placeholders", () => {
    expect(() => renderServiceWorker("self.x = 1", SHELL, "v")).toThrow()
  })
})

describe("service worker", () => {
  let worker: ReturnType<typeof loadWorker>

  beforeEach(async () => {
    worker = loadWorker("v2", ["ferrofolio-shell-v1", "unrelated"])
    await runLifecycle(worker.handlers.install)
  })

  it("precaches the initial shell on install, bypassing the HTTP cache", () => {
    expect([...worker.cache().keys()]).toEqual(SHELL.precache)
  })

  it("does not activate a new version on its own", () => {
    expect(worker.self.skipWaiting).not.toHaveBeenCalled()
  })

  it("activates when the page asks for it", () => {
    worker.handlers.message({ data: { type: "SKIP_WAITING" } })
    expect(worker.self.skipWaiting).toHaveBeenCalledOnce()
  })

  it("removes caches of earlier versions only", async () => {
    await runLifecycle(worker.handlers.activate)
    expect([...worker.stored.keys()].sort()).toEqual([
      "ferrofolio-shell-v2",
      "unrelated",
    ])
  })

  it("serves shell files from the cache", async () => {
    const response = await dispatchFetch(worker.handlers.fetch, {
      url: `${ORIGIN}/assets/index-abc.js`,
    })
    expect(response).toBe("body of /assets/index-abc.js")
    expect(worker.fetchMock).not.toHaveBeenCalled()
  })

  it("serves navigations to the app from the cached index", async () => {
    const response = await dispatchFetch(worker.handlers.fetch, {
      url: `${ORIGIN}/index.html?x=1`,
      mode: "navigate",
    })
    expect(response).toBe("body of /")
  })

  it("caches a lazy chunk of the shell on first use", async () => {
    const request = { url: `${ORIGIN}/assets/lazy-abc.js` }
    const first: any = await dispatchFetch(worker.handlers.fetch, request)
    expect(first.body).toBe("from network")
    expect(worker.cache().has("/assets/lazy-abc.js")).toBe(true)

    await dispatchFetch(worker.handlers.fetch, request)
    expect(worker.fetchMock).toHaveBeenCalledOnce()
  })

  it("returns the network response when the cache cannot store it", async () => {
    const request = { url: `${ORIGIN}/assets/lazy-abc.js` }
    worker.failPut()
    const response: any = await dispatchFetch(worker.handlers.fetch, request)
    expect(response.body).toBe("from network")
    expect(worker.cache().has("/assets/lazy-abc.js")).toBe(false)
  })

  it("does not cache a failed response", async () => {
    worker.fetchMock.mockResolvedValueOnce(fakeResponse("missing", false))
    await dispatchFetch(worker.handlers.fetch, {
      url: `${ORIGIN}/assets/lazy-abc.js`,
    })
    expect(worker.cache().has("/assets/lazy-abc.js")).toBe(false)
  })

  it.each([
    ["an API request", `${ORIGIN}/api/v1/positions`, "GET", "cors"],
    ["a file outside the shell", `${ORIGIN}/icons/n26.png`, "GET", "no-cors"],
    ["the runtime configuration", `${ORIGIN}/env-config.js`, "GET", "no-cors"],
    [
      "another origin",
      "https://api.example/assets/index-abc.js",
      "GET",
      "cors",
    ],
    ["a write", `${ORIGIN}/assets/index-abc.js`, "POST", "cors"],
    ["a navigation elsewhere", `${ORIGIN}/api/v1/status`, "GET", "navigate"],
  ])("leaves %s to the network", async (_name, url, method, mode) => {
    const response = await dispatchFetch(worker.handlers.fetch, {
      url,
      method,
      mode,
    })
    expect(response).toBeUndefined()
    expect(worker.fetchMock).not.toHaveBeenCalled()
  })
})
