import { afterEach, describe, expect, it, vi } from "vitest"
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"

import { CloudRestore } from "@/components/auth/CloudRestore"
import { CloudTab } from "@/components/settings/CloudTab"
import { ModalRegistryProvider } from "@/context/ModalRegistryContext"
import en from "@/i18n/locales/en.json"
import { getBackupsInfoWithCloudAuth } from "@/services/api"
import type { FullBackupsInfo } from "@/types"

const { signInWithEmail, getSession } = vi.hoisted(() => ({
  signInWithEmail: vi.fn().mockResolvedValue(undefined),
  getSession: vi.fn().mockResolvedValue(null),
}))

vi.mock("@/i18n", () => ({ useI18n: () => ({ t: en, locale: "en-US" }) }))
vi.mock("@/context/CloudContext", () => ({
  useCloud: () => ({
    user: null,
    role: null,
    permissions: [],
    isLoading: false,
    isInitialized: true,
    signInWithEmail,
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
    signInWithEmail = signInWithEmail
    getSession = getSession
  },
}))
vi.mock("@/components/telemetry/ErrorReportingToggle", () => ({
  ErrorReportingToggle: () => null,
}))

afterEach(() => {
  cleanup()
  signInWithEmail.mockClear()
  getSession.mockClear()
})

function submitEmailSignIn() {
  fireEvent.change(
    screen.getByPlaceholderText(en.settings.cloud.emailPlaceholder),
    { target: { value: "user@example.com" } },
  )
  fireEvent.change(
    screen.getByPlaceholderText(en.settings.cloud.passwordPlaceholder),
    { target: { value: "example-password" } },
  )
  fireEvent.click(
    screen.getByRole("button", { name: en.settings.cloud.signInWithEmail }),
  )
}

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
    () => render(<CloudRestore onBack={vi.fn()} onRestoreComplete={vi.fn()} />),
  ],
] as const

describe.each(surfaces)("cloud sign-in in %s", (_name, renderSurface) => {
  it("offers email sign-in only", () => {
    renderSurface()

    expect(
      screen.getByRole("button", { name: en.settings.cloud.signInWithEmail }),
    ).toBeInTheDocument()
    expect(
      screen.queryByRole("button", { name: /google/i }),
    ).not.toBeInTheDocument()
    expect(
      screen.queryByRole("button", { name: /apple/i }),
    ).not.toBeInTheDocument()
    expect(screen.queryByText(/google/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/apple/i)).not.toBeInTheDocument()
  })

  it("submits the email sign-in form", async () => {
    renderSurface()

    submitEmailSignIn()

    await waitFor(() =>
      expect(signInWithEmail).toHaveBeenCalledWith(
        "user@example.com",
        "example-password",
      ),
    )
  })
})

describe("cloud restore after email sign-in", () => {
  it("asks for the restore credentials when a backup exists", async () => {
    getSession.mockResolvedValueOnce({
      accessToken: "access-token",
      refreshToken: "refresh-token",
      tokenType: "bearer",
      expiresAt: 0,
      user: { id: "user-1", email: "user@example.com" },
    })
    vi.mocked(getBackupsInfoWithCloudAuth).mockResolvedValueOnce({
      pieces: { DATA: { local: null, remote: {} } },
    } as unknown as FullBackupsInfo)
    render(<CloudRestore onBack={vi.fn()} onRestoreComplete={vi.fn()} />)

    submitEmailSignIn()

    expect(
      await screen.findByText(en.login.cloudRestore.backupFound),
    ).toBeInTheDocument()
    expect(getBackupsInfoWithCloudAuth).toHaveBeenCalledWith("access-token")
    expect(
      screen.getByPlaceholderText(
        en.login.cloudRestore.encryptionKeyPlaceholder,
      ),
    ).toBeInTheDocument()
  })
})
