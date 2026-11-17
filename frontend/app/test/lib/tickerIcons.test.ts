// SPDX-License-Identifier: AGPL-3.0-only
import { afterEach, describe, expect, it, vi } from "vitest"

import { tickerIconUrl } from "@/lib/tickerIcons"

describe("tickerIconUrl", () => {
  afterEach(() => {
    vi.unstubAllEnvs()
    vi.resetModules()
  })

  it("loads no icon with the default configuration", async () => {
    vi.stubEnv("VITE_TICKER_ICON_URL", "")
    vi.resetModules()
    const { tickerIconUrl: withDefault } = await import("@/lib/tickerIcons")

    expect(withDefault("AAPL")).toBeNull()
  })

  it("loads no icon without a configured source", () => {
    expect(tickerIconUrl("US0378331005", "")).toBeNull()
    expect(tickerIconUrl("AAPL", "")).toBeNull()
  })

  it("builds the icon URL from the configured source", () => {
    expect(tickerIconUrl(" AAPL ", "https://icons.example.com/ticker/")).toBe(
      "https://icons.example.com/ticker/AAPL.png",
    )
  })

  it("encodes the token", () => {
    expect(tickerIconUrl("BRK/B", "https://icons.example.com")).toBe(
      "https://icons.example.com/BRK%2FB.png",
    )
  })

  it("returns null for an empty token", () => {
    expect(tickerIconUrl("  ", "https://icons.example.com")).toBeNull()
    expect(tickerIconUrl(null, "https://icons.example.com")).toBeNull()
  })
})
