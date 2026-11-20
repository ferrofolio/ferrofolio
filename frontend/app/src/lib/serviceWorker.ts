// Registers the service worker of the app shell (pwa/sw.js) and tracks whether a new version is
// waiting. A new version is never activated silently: the user confirms the reload.

const UPDATE_CHECK_INTERVAL_MS = 60 * 60 * 1000

type Listener = (updateAvailable: boolean) => void

let waitingWorker: ServiceWorker | null = null
let updateRequested = false
const listeners = new Set<Listener>()

function setWaitingWorker(worker: ServiceWorker | null): void {
  waitingWorker = worker
  listeners.forEach(listener => listener(worker !== null))
}

export function subscribeToServiceWorkerUpdate(listener: Listener): () => void {
  listeners.add(listener)
  listener(waitingWorker !== null)
  return () => {
    listeners.delete(listener)
  }
}

// Activates the waiting version; the page reloads once it controls the page.
export function applyServiceWorkerUpdate(): void {
  if (!waitingWorker) return
  updateRequested = true
  waitingWorker.postMessage({ type: "SKIP_WAITING" })
}

function trackInstalling(
  registration: ServiceWorkerRegistration,
  container: ServiceWorkerContainer,
): void {
  const installing = registration.installing
  if (!installing) return
  installing.addEventListener("statechange", () => {
    // Without a controller this is the first installation, not an update.
    if (installing.state === "installed" && container.controller) {
      setWaitingWorker(installing)
    }
  })
}

export async function registerServiceWorker(
  container: ServiceWorkerContainer | undefined = typeof navigator !==
  "undefined"
    ? navigator.serviceWorker
    : undefined,
  reload: () => void = () => window.location.reload(),
): Promise<void> {
  // Service workers need a secure context (HTTPS or localhost); elsewhere the app runs without.
  if (!container) return

  container.addEventListener("controllerchange", () => {
    if (!updateRequested) return
    updateRequested = false
    reload()
  })

  const registration = await container.register("/sw.js")
  if (registration.waiting && container.controller) {
    setWaitingWorker(registration.waiting)
  }
  trackInstalling(registration, container)
  registration.addEventListener("updatefound", () =>
    trackInstalling(registration, container),
  )

  const checkForUpdate = () => {
    registration.update().catch(() => undefined)
  }
  setInterval(checkForUpdate, UPDATE_CHECK_INTERVAL_MS)
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") checkForUpdate()
  })
}

export function resetServiceWorkerStateForTests(): void {
  waitingWorker = null
  updateRequested = false
  listeners.clear()
}
