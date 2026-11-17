// SPDX-License-Identifier: AGPL-3.0-only
import { describe, expect, it } from "vitest"

import { tickerIconUrl } from "@/lib/tickerIcons"

describe("tickerIconUrl", () => {
  it("loads no icon without a configured source", () => {
    expect(tickerIconUrl("US0378331005")).toBeNull()
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
