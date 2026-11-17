import { afterEach, describe, expect, it } from "vitest"

import {
  getPlatformInfo,
  getPlatformType,
  isElectron,
  isWeb,
} from "@/lib/platform"
import { PlatformType } from "@/types"

afterEach(() => {
  delete (window as { platform?: unknown }).platform
  delete (window as { ipcAPI?: unknown }).ipcAPI
})

describe("platform detection", () => {
  it("defaults to the web platform when nothing is injected", () => {
    expect(getPlatformInfo()).toEqual({ type: PlatformType.WEB })
    expect(getPlatformType()).toBe(PlatformType.WEB)
    expect(isElectron()).toBe(false)
    expect(isWeb()).toBe(true)
  })

  it.each([
    PlatformType.WEB,
    PlatformType.IOS,
    PlatformType.ANDROID,
    PlatformType.WINDOWS,
  ])("treats %s without the Electron bridge as web", type => {
    window.platform = { type }

    expect(getPlatformType()).toBe(type)
    expect(isElectron()).toBe(false)
    expect(isWeb()).toBe(true)
  })

  it.each([
    PlatformType.WINDOWS,
    PlatformType.MAC,
    PlatformType.LINUX,
    PlatformType.IOS,
    PlatformType.ANDROID,
  ])("treats %s with the Electron bridge as Electron", type => {
    window.platform = { type }
    window.ipcAPI = {} as typeof window.ipcAPI

    expect(isElectron()).toBe(true)
    expect(isWeb()).toBe(false)
  })
})
