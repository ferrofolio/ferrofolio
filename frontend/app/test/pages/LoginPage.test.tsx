import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, render, screen } from "@testing-library/react"
import LoginPage from "@/pages/LoginPage"
import en from "@/i18n/locales/en.json"

const auth = vi.hoisted(() => ({
  apiSessionEnabled: false,
  isChangingPassword: false,
}))

const mockUseAuth = vi.hoisted(() => ({
  useAuth: () => ({
    login: vi.fn(),
    logout: vi.fn(),
    isAuthenticated: false,
    user: null,
    ...auth,
  }),
}))

vi.mock("@/context/AuthContext", () => mockUseAuth)
vi.mock("@/i18n", () => ({ useI18n: () => ({ t: en, locale: "en-US" }) }))
vi.mock("@/context/ThemeContext", () => ({
  useTheme: () => ({ theme: "dark" }),
}))
vi.mock("@/context/AppContext", () => ({
  useAppContext: () => ({
    setBackendConfig: vi.fn(),
    backendConfig: { host: "http://localhost:7592", port: 7592 },
  }),
}))
vi.mock("@/services/api", () => ({
  getApiServerInfo: vi.fn(() => Promise.resolve({ status: "ok" })),
  checkStatus: vi.fn(() => Promise.resolve(true)),
}))
vi.mock("@/context/featureFlagsStore", () => ({
  setFeatureFlags: vi.fn(),
}))
vi.mock("@/hooks/useModalBackHandler", () => ({
  useModalBackHandler: () => {},
}))

describe("LoginPage", () => {
  afterEach(() => {
    cleanup()
    auth.apiSessionEnabled = false
    auth.isChangingPassword = false
  })

  it("renders the ferrofolio-fg.svg logo without invert", () => {
    render(<LoginPage />)

    const logo = screen.getByAltText(en.common.logoAlt) as HTMLImageElement
    expect(logo.getAttribute("src")).toBe("ferrofolio-fg.svg")
    expect(logo.className).not.toContain("invert")
  })

  it.each([
    { apiSessionEnabled: true, isChangingPassword: false, shown: true },
    { apiSessionEnabled: false, isChangingPassword: false, shown: false },
    { apiSessionEnabled: true, isChangingPassword: true, shown: false },
  ])(
    "shows the remember-session option: $shown (sessions: $apiSessionEnabled, changing password: $isChangingPassword)",
    ({ apiSessionEnabled, isChangingPassword, shown }) => {
      auth.apiSessionEnabled = apiSessionEnabled
      auth.isChangingPassword = isChangingPassword

      render(<LoginPage />)

      const option = screen.queryByLabelText(en.login.rememberSession)
      if (shown) expect(option).toBeInTheDocument()
      else expect(option).not.toBeInTheDocument()
    },
  )
})
