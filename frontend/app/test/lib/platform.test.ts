import { afterEach, describe, expect, it, vi } from "vitest"

import { isPWAStandalone } from "@/lib/platform"

function mockDisplayMode(standalone: boolean) {
  vi.stubGlobal(
    "matchMedia",
    vi.fn((query: string) => ({
      matches: standalone && query === "(display-mode: standalone)",
    })),
  )
}

afterEach(() => {
  vi.unstubAllGlobals()
  delete (navigator as { standalone?: boolean }).standalone
})

describe("isPWAStandalone", () => {
  it("is false in a browser tab", () => {
    mockDisplayMode(false)

    expect(isPWAStandalone()).toBe(false)
  })

  it("is true in the standalone display mode", () => {
    mockDisplayMode(true)

    expect(isPWAStandalone()).toBe(true)
  })

  it("is true when Safari reports a home screen launch", () => {
    mockDisplayMode(false)
    Object.defineProperty(navigator, "standalone", {
      value: true,
      configurable: true,
    })

    expect(isPWAStandalone()).toBe(true)
  })
})
