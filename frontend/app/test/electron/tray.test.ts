// @vitest-environment node
// SPDX-License-Identifier: AGPL-3.0-only
import { beforeEach, describe, expect, it, vi } from "vitest"
import type { Tray } from "electron"

const { Menu: MockMenu, nativeImage: mockNativeImage } = vi.hoisted(() => ({
  Menu: {
    buildFromTemplate: vi.fn(template => ({ items: template })),
  },
  nativeImage: {
    createFromPath: vi.fn(() => ({
      isEmpty: () => false,
      setTemplateImage: vi.fn(),
    })),
  },
}))
const mockTray = {
  setToolTip: vi.fn(),
  setContextMenu: vi.fn(),
  on: vi.fn(),
  destroy: vi.fn(),
} as unknown as Tray

vi.mock("electron", () => ({
  app: { quit: vi.fn(), getPath: vi.fn(() => "/mock") },
  BrowserWindow: { getAllWindows: () => [] },
  ipcMain: { handle: vi.fn() },
  Menu: MockMenu,
  nativeImage: mockNativeImage,
  Tray: vi.fn(function () {
    return mockTray
  }),
}))

vi.mock("../../electron/main/windows-process", () => ({
  findAndKillProcesses: vi.fn(),
}))
vi.mock("../../electron/main/logging", () => ({ rawLog: vi.fn() }))

let trayModule: typeof import("../../electron/main/tray")

async function loadModule() {
  vi.resetModules()
  trayModule = await import("../../electron/main/tray")
  return trayModule
}

describe("system tray", () => {
  beforeEach(async () => {
    vi.clearAllMocks()
  })

  it("creates menu items labelled Ferrofolio with Ferrofolio tooltip", async () => {
    const mod = await loadModule()

    const onShow = vi.fn()
    const onAbout = vi.fn()
    const tray = mod.createTray({
      publicPath: "/mock",
      onShowWindow: onShow,
      onAbout,
    })

    expect(MockMenu.buildFromTemplate).toHaveBeenCalledTimes(1)
    const template = MockMenu.buildFromTemplate.mock.calls[0][0] as {
      label: string
      type?: string
      click?: () => void
    }[]

    const items = template.filter(i => i.type === "separator").length
    expect(items).toBe(2)
    expect(template.filter(i => i.label === "Ferrofolio").length).toBe(1)
    expect(template.filter(i => i.label === "Quit").length).toBe(1)
    expect(template.filter(i => i.label === "About").length).toBe(1)

    expect(mockTray.setToolTip).toHaveBeenCalledWith("Ferrofolio")

    // Click the Ferrofolio menu item
    const ferroItem = template.find(i => i.label === "Ferrofolio")!
    await ferroItem.click!()
    expect(onShow).toHaveBeenCalledOnce()

    expect(tray).toBeDefined()
  })
})
