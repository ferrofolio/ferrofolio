import { act, cleanup, render } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { AuthProvider, useAuth } from "@/context/AuthContext"

const { checkStatusMock } = vi.hoisted(() => ({
  checkStatusMock: vi.fn(),
}))

vi.mock("@/services/api", () => ({
  checkStatus: checkStatusMock,
  login: vi.fn(),
  logout: vi.fn(),
  signup: vi.fn(),
  changePassword: vi.fn(),
}))

vi.mock("@/hooks/useBackupStatus", () => ({
  resetBackupStatusCache: vi.fn(),
}))

let observed = { hasConnectionFailed: false, isInitializing: true }

function Probe() {
  const { hasConnectionFailed, isInitializing } = useAuth()
  observed = { hasConnectionFailed, isInitializing }
  return null
}

describe("AuthProvider startup connection handling", () => {
  beforeEach(() => {
    cleanup()
    vi.useFakeTimers()
    vi.clearAllMocks()
    observed = { hasConnectionFailed: false, isInitializing: true }
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it("flags the first failure, keeps initializing while retrying, and clears both after success", async () => {
    checkStatusMock
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce({
        status: "LOCKED",
        lastLogged: null,
        features: {},
        user: null,
        pendingRegister: false,
        apiSession: false,
      })

    render(
      <AuthProvider>
        <Probe />
      </AuthProvider>,
    )

    await act(async () => {
      await Promise.resolve()
    })

    expect(checkStatusMock).toHaveBeenCalledTimes(1)
    expect(observed.hasConnectionFailed).toBe(true)
    expect(observed.isInitializing).toBe(true)

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1500)
    })

    expect(checkStatusMock).toHaveBeenCalledTimes(2)
    expect(observed.hasConnectionFailed).toBe(false)
    expect(observed.isInitializing).toBe(false)
  })
})
