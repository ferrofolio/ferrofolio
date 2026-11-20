import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

const { authState, regionRestricted } = vi.hoisted(() => ({
  authState: {
    isAuthenticated: false,
    isInitializing: true,
    hasConnectionFailed: false,
  },
  regionRestricted: { value: null as boolean | null },
}))

vi.mock("@/context/AuthContext", () => ({
  useAuth: () => authState,
}))

vi.mock("@/hooks/useRegionRestriction", () => ({
  useRegionRestriction: () => regionRestricted.value,
}))

vi.mock("@/hooks/useReleaseUpdate", () => ({
  useReleaseUpdate: () => ({ updateInfo: {} }),
}))

vi.mock("@/components/SplashScreen", () => ({
  default: ({ hasConnectionFailed }: { hasConnectionFailed?: boolean }) => (
    <div data-testid="splash" data-failed={String(hasConnectionFailed)} />
  ),
}))

import App from "@/App"

describe("App splash screen connection state", () => {
  beforeEach(() => {
    cleanup()
    authState.isAuthenticated = false
    authState.isInitializing = true
    authState.hasConnectionFailed = false
    regionRestricted.value = null
  })

  afterEach(() => {
    cleanup()
  })

  it("passes hasConnectionFailed to the splash screen while initializing", () => {
    authState.isInitializing = true
    authState.hasConnectionFailed = true

    render(<App />)

    expect(screen.getByTestId("splash")).toHaveAttribute("data-failed", "true")
  })

  it("passes hasConnectionFailed to the splash screen while the region is unknown", () => {
    authState.isInitializing = false
    authState.isAuthenticated = false
    authState.hasConnectionFailed = true
    regionRestricted.value = null

    render(<App />)

    expect(screen.getByTestId("splash")).toHaveAttribute("data-failed", "true")
  })
})
