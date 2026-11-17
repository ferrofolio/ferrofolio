import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, render, screen } from "@testing-library/react"

import { CloudRestore } from "@/components/auth/CloudRestore"
import { CloudTab } from "@/components/settings/CloudTab"
import { ModalRegistryProvider } from "@/context/ModalRegistryContext"
import en from "@/i18n/locales/en.json"
import { PlatformType } from "@/types"

vi.mock("@/i18n", () => ({ useI18n: () => ({ t: en, locale: "en-US" }) }))
vi.mock("@/context/CloudContext", () => ({
  useCloud: () => ({
    user: null,
    role: null,
    permissions: [],
    isLoading: false,
    isInitialized: true,
    oauthError: null,
    clearOAuthError: vi.fn(),
    signInWithGoogle: vi.fn(),
    signInWithApple: vi.fn(),
    signInWithEmail: vi.fn(),
    signUpWithEmail: vi.fn(),
    requestPasswordReset: vi.fn(),
    isPasswordRecoveryActive: false,
    clearPasswordRecovery: vi.fn(),
    updatePassword: vi.fn(),
    signOut: vi.fn(),
  }),
}))
vi.mock("@/hooks/useBackupStatus", () => ({ useBackupStatus: () => ({}) }))
vi.mock("@/context/AuthContext", () => ({
  useAuth: () => ({
    guestSignup: vi.fn(),
    signup: vi.fn(),
    logout: vi.fn(),
  }),
}))
vi.mock("@/context/ThemeContext", () => ({
  useTheme: () => ({ theme: "dark" }),
}))
vi.mock("@/services/api", () => ({
  getBackupsInfoWithCloudAuth: vi.fn(),
  importBackup: vi.fn(),
  cloudAuth: vi.fn(),
  getApiServerInfo: vi.fn(),
}))
vi.mock("@/services/cloud/supabaseProvider", () => ({
  SupabaseAuthProvider: class {
    initialize = vi.fn().mockResolvedValue(undefined)
    onAuthStateChange = vi.fn(() => () => {})
  },
}))
vi.mock("@/components/telemetry/ErrorReportingToggle", () => ({
  ErrorReportingToggle: () => null,
}))

afterEach(() => {
  cleanup()
  delete (window as { platform?: unknown }).platform
  delete (window as { ipcAPI?: unknown }).ipcAPI
})

const surfaces = [
  [
    "CloudTab",
    () =>
      render(
        <ModalRegistryProvider>
          <CloudTab />
        </ModalRegistryProvider>,
      ),
  ],
  [
    "CloudRestore",
    () =>
      render(
        <CloudRestore
          onBack={vi.fn()}
          onRestoreComplete={vi.fn()}
          isDesktop={false}
        />,
      ),
  ],
] as const

describe.each(surfaces)("social sign-in in %s", (_name, renderSurface) => {
  it.each([PlatformType.WEB, PlatformType.IOS, PlatformType.ANDROID])(
    "is unavailable on %s without the Electron bridge",
    type => {
      window.platform = { type }
      renderSurface()

      expect(
        screen.getByRole("button", {
          name: en.settings.cloud.signInWithGoogle,
        }),
      ).toBeDisabled()
      expect(
        screen.getByText(en.settings.cloud.googleDesktopOnly),
      ).toBeInTheDocument()
      expect(
        screen.queryByRole("button", {
          name: en.settings.cloud.signInWithApple,
        }),
      ).not.toBeInTheDocument()
    },
  )

  it.each([PlatformType.WINDOWS, PlatformType.MAC, PlatformType.LINUX])(
    "is available on %s with the Electron bridge",
    type => {
      window.platform = { type }
      window.ipcAPI = {} as typeof window.ipcAPI
      renderSurface()

      expect(
        screen.getByRole("button", {
          name: en.settings.cloud.signInWithGoogle,
        }),
      ).toBeEnabled()
      expect(
        screen.queryByText(en.settings.cloud.googleDesktopOnly),
      ).not.toBeInTheDocument()
      expect(
        screen.getByRole("button", { name: en.settings.cloud.signInWithApple }),
      ).toBeEnabled()
    },
  )
})
