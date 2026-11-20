import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import {
  applyServiceWorkerUpdate,
  registerServiceWorker,
  resetServiceWorkerStateForTests,
  subscribeToServiceWorkerUpdate,
} from "@/lib/serviceWorker"

class FakeWorker extends EventTarget {
  state = "installing"
  postMessage = vi.fn()

  becomeInstalled() {
    this.state = "installed"
    this.dispatchEvent(new Event("statechange"))
  }
}

class FakeRegistration extends EventTarget {
  installing: FakeWorker | null = null
  waiting: FakeWorker | null = null
  update = vi.fn(async () => undefined)
}

class FakeContainer extends EventTarget {
  controller: object | null = null
  registration = new FakeRegistration()
  register = vi.fn(async () => this.registration)
}

function setup(controlled: boolean) {
  const container = new FakeContainer()
  container.controller = controlled ? {} : null
  const reload = vi.fn()
  const updates: boolean[] = []
  subscribeToServiceWorkerUpdate(available => updates.push(available))
  return { container, reload, updates }
}

describe("registerServiceWorker", () => {
  beforeEach(() => {
    vi.useFakeTimers()
    resetServiceWorkerStateForTests()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it("does nothing without service worker support", async () => {
    await expect(registerServiceWorker(undefined)).resolves.toBeUndefined()
  })

  it("registers the worker at the root", async () => {
    const { container, reload } = setup(false)
    await registerServiceWorker(container as any, reload)
    expect(container.register).toHaveBeenCalledWith("/sw.js")
  })

  it("does not offer an update on the first installation", async () => {
    const { container, reload, updates } = setup(false)
    await registerServiceWorker(container as any, reload)
    const worker = new FakeWorker()
    container.registration.installing = worker
    container.registration.dispatchEvent(new Event("updatefound"))
    worker.becomeInstalled()
    expect(updates).toEqual([false])
  })

  it("offers an update once a new version is installed", async () => {
    const { container, reload, updates } = setup(true)
    await registerServiceWorker(container as any, reload)
    const worker = new FakeWorker()
    container.registration.installing = worker
    container.registration.dispatchEvent(new Event("updatefound"))
    worker.becomeInstalled()
    expect(updates).toEqual([false, true])
  })

  it("offers an update that was already waiting", async () => {
    const { container, reload, updates } = setup(true)
    container.registration.waiting = new FakeWorker()
    await registerServiceWorker(container as any, reload)
    expect(updates).toEqual([false, true])
  })

  it("activates the update and reloads only when the user asks", async () => {
    const { container, reload } = setup(true)
    const waiting = new FakeWorker()
    waiting.state = "installed"
    container.registration.waiting = waiting
    await registerServiceWorker(container as any, reload)

    container.dispatchEvent(new Event("controllerchange"))
    expect(reload).not.toHaveBeenCalled()

    applyServiceWorkerUpdate()
    expect(waiting.postMessage).toHaveBeenCalledWith({ type: "SKIP_WAITING" })
    container.dispatchEvent(new Event("controllerchange"))
    container.dispatchEvent(new Event("controllerchange"))
    expect(reload).toHaveBeenCalledOnce()
  })

  it("reloads at once when another tab has already activated the update", async () => {
    const { container, reload } = setup(true)
    const waiting = new FakeWorker()
    waiting.state = "installed"
    container.registration.waiting = waiting
    await registerServiceWorker(container as any, reload)

    // Another tab accepted the update: the worker activates and takes over this page too.
    waiting.state = "activated"
    container.dispatchEvent(new Event("controllerchange"))
    expect(reload).not.toHaveBeenCalled()

    applyServiceWorkerUpdate()
    expect(waiting.postMessage).not.toHaveBeenCalled()
    expect(reload).toHaveBeenCalledOnce()
  })

  it("checks for updates every hour", async () => {
    const { container, reload } = setup(true)
    await registerServiceWorker(container as any, reload)
    expect(container.registration.update).not.toHaveBeenCalled()
    vi.advanceTimersByTime(60 * 60 * 1000)
    expect(container.registration.update).toHaveBeenCalledOnce()
  })
})
