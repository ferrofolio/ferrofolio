// @vitest-environment node
// SPDX-License-Identifier: AGPL-3.0-only
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { OS } from "../../electron/types"

const { autoUpdater } = vi.hoisted(() => ({
  autoUpdater: {
    channel: "",
    setFeedURL: vi.fn(),
    checkForUpdates: vi.fn(() => Promise.resolve(null)),
    on: vi.fn(),
  },
}))

vi.mock("electron", () => ({
  BrowserWindow: { getAllWindows: () => [] },
  ipcMain: { handle: vi.fn() },
}))
vi.mock("electron-updater", () => ({ default: { autoUpdater } }))
vi.mock("../../electron/main/telemetry", () => ({ captureException: vi.fn() }))

const appConfig = (isDev: boolean) => ({
  isDev,
  os: OS.LINUX,
  arch: "x64",
  ports: { backend: 7592 },
  urls: { backend: "http://localhost:7592", vite: "" },
})

async function loadUpdater() {
  vi.resetModules()
  return import("../../electron/main/auto-updater")
}

describe("auto updater", () => {
  beforeEach(() => {
    vi.clearAllMocks()
    delete process.env.AUTO_UPDATE_FEED_URL
  })

  afterEach(() => {
    delete process.env.AUTO_UPDATE_FEED_URL
  })

  it("does not check for updates without a configured feed", async () => {
    const updater = await loadUpdater()

    updater.setupAutoUpdater(appConfig(false) as never)
    updater.checkForUpdatesOnStartup()

    expect(updater.isAutoUpdateSupported()).toBe(false)
    expect(autoUpdater.setFeedURL).not.toHaveBeenCalled()
    expect(autoUpdater.checkForUpdates).not.toHaveBeenCalled()
  })

  it("uses an explicitly configured feed", async () => {
    process.env.AUTO_UPDATE_FEED_URL = "https://updates.example.com"
    const updater = await loadUpdater()

    updater.setupAutoUpdater(appConfig(false) as never)
    updater.checkForUpdatesOnStartup()

    expect(updater.isAutoUpdateSupported()).toBe(true)
    expect(autoUpdater.setFeedURL).toHaveBeenCalledWith({
      provider: "generic",
      url: "https://updates.example.com",
    })
    expect(autoUpdater.checkForUpdates).toHaveBeenCalledOnce()
  })

  it("never updates development builds", async () => {
    process.env.AUTO_UPDATE_FEED_URL = "https://updates.example.com"
    const updater = await loadUpdater()

    updater.setupAutoUpdater(appConfig(true) as never)

    expect(updater.isAutoUpdateSupported()).toBe(false)
  })
})
