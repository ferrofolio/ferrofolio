import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, render, screen } from "@testing-library/react"
import SplashScreen from "@/components/SplashScreen"

vi.mock("@/i18n", () => ({
  useI18n: () => ({
    t: {
      common: {
        logoAlt: "Ferrofolio logo",
        connectingToServer: "Connecting to the server...",
      },
      splash: { loading: "Loading..." },
    },
    locale: "en-US",
  }),
}))

vi.mock("@/context/ThemeContext", () => ({
  useTheme: () => ({ theme: "dark" }),
}))

vi.mock("@/services/api", () => ({
  getApiServerInfo: () => new Promise(() => {}),
}))

describe("SplashScreen", () => {
  afterEach(() => {
    cleanup()
  })

  it("renders the ferrofolio-fg.svg logo without invert", () => {
    render(<SplashScreen />)

    const img = screen.getByAltText("Ferrofolio logo") as HTMLImageElement
    expect(img.src).toContain("ferrofolio-fg.svg")
    expect(img.className).not.toContain("invert")
  })

  it("shows that it is connecting to the server", () => {
    render(<SplashScreen />)

    expect(screen.getByText("Connecting to the server...")).toBeInTheDocument()
  })
})
