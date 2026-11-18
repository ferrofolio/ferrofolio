import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, render, screen } from "@testing-library/react"
import { RestrictedRegionScreen } from "@/components/auth/RestrictedRegionScreen"

vi.mock("@/i18n", () => ({
  useI18n: () => ({
    t: {
      common: { logoAlt: "Ferrofolio logo" },
      login: {
        regionRestricted: {
          title: "Region Restricted",
          description: "Restricted",
        },
      },
    },
    locale: "en-US",
  }),
}))

vi.mock("@/context/ThemeContext", () => ({
  useTheme: () => ({ theme: "dark" }),
}))

describe("RestrictedRegionScreen", () => {
  afterEach(() => {
    cleanup()
  })

  it("renders the ferrofolio-fg.svg logo without invert", () => {
    render(<RestrictedRegionScreen />)

    const img = screen.getByAltText("Ferrofolio logo") as HTMLImageElement
    expect(img.src).toContain("ferrofolio-fg.svg")
    expect(img.className).not.toContain("invert")
  })
})
