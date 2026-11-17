import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { openExternalAuthLink, openExternalAuthPage } from "@/lib/externalAuth"
import { PlatformType } from "@/types"

const LINK = "https://auth.example.com/authorize?state=s1"

const assign = vi.fn()
const open = vi.fn()

beforeEach(() => {
  assign.mockReset()
  open.mockReset()
  vi.stubGlobal("location", { ...window.location, assign })
  vi.spyOn(window, "open").mockImplementation(open)
})

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
  delete (window as { platform?: unknown }).platform
  delete (window as { ipcAPI?: unknown }).ipcAPI
})

function usePlatform(type: PlatformType, electron: boolean) {
  window.platform = { type }
  if (electron) window.ipcAPI = {} as typeof window.ipcAPI
}

describe("openExternalAuthLink", () => {
  it.each([PlatformType.WEB, PlatformType.IOS, PlatformType.ANDROID])(
    "navigates the current tab on %s without Electron",
    async type => {
      usePlatform(type, false)

      await openExternalAuthLink(LINK)

      expect(assign).toHaveBeenCalledExactlyOnceWith(LINK)
      expect(open).not.toHaveBeenCalled()
    },
  )

  it.each([PlatformType.WINDOWS, PlatformType.MAC, PlatformType.LINUX])(
    "opens a new window on %s in Electron",
    async type => {
      usePlatform(type, true)

      await openExternalAuthLink(LINK)

      expect(open).toHaveBeenCalledExactlyOnceWith(LINK, "_blank")
      expect(assign).not.toHaveBeenCalled()
    },
  )

  it("ignores a failing window open in Electron", async () => {
    usePlatform(PlatformType.LINUX, true)
    open.mockImplementation(() => {
      throw new Error("blocked")
    })

    await expect(openExternalAuthLink(LINK)).resolves.toBeUndefined()
  })
})

describe("openExternalAuthPage", () => {
  it.each([
    [PlatformType.WEB, false],
    [PlatformType.IOS, false],
    [PlatformType.ANDROID, false],
    [PlatformType.WINDOWS, true],
    [PlatformType.MAC, true],
    [PlatformType.LINUX, true],
  ])("opens a detached window on %s (Electron: %s)", async (type, electron) => {
    usePlatform(type, electron)

    await openExternalAuthPage(LINK)

    expect(open).toHaveBeenCalledExactlyOnceWith(
      LINK,
      "_blank",
      "noopener,noreferrer",
    )
    expect(assign).not.toHaveBeenCalled()
  })

  it("ignores a failing window open", async () => {
    open.mockImplementation(() => {
      throw new Error("blocked")
    })

    await expect(openExternalAuthPage(LINK)).resolves.toBeUndefined()
  })
})
