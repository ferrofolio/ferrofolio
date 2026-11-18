import {
  createContext,
  useContext,
  useState,
  useEffect,
  useCallback,
  useRef,
  type ReactNode,
} from "react"
import { useNavigate } from "react-router-dom"
import { BackupMode, CloudRole, FFStatus } from "@/types"
import {
  cloudAuth,
  getCloudAuthToken,
  getBackupSettings,
  updateBackupSettings,
} from "@/services/api"
import {
  CloudAuthProvider,
  CloudSession,
  CloudUser,
  SupabaseAuthProvider,
  type EmailPasswordSignUpResult,
} from "@/services/cloud"
import { useAuth } from "@/context/AuthContext"
import { useAppContext } from "@/context/AppContext"
import { resetBackupStatusCache } from "@/hooks/useBackupStatus"

interface CloudContextType {
  user: CloudUser | null
  role: CloudRole | null
  permissions: string[]
  backupMode: BackupMode
  setBackupMode: (mode: BackupMode) => void
  isLoading: boolean
  isInitialized: boolean
  isPasswordRecoveryActive: boolean
  clearPasswordRecovery: () => void
  signInWithEmail: (email: string, password: string) => Promise<void>
  signUpWithEmail: (
    email: string,
    password: string,
  ) => Promise<EmailPasswordSignUpResult>
  requestPasswordReset: (email: string) => Promise<void>
  updatePassword: (password: string) => Promise<void>
  signOut: () => Promise<void>
}

const CloudContext = createContext<CloudContextType | undefined>(undefined)

const createAuthProvider = (): CloudAuthProvider => {
  return new SupabaseAuthProvider()
}

export function CloudProvider({ children }: { children: ReactNode }) {
  const navigate = useNavigate()
  const { isAuthenticated } = useAuth()
  const { featureFlags } = useAppContext()

  const isCloudEnabled = featureFlags.CLOUD === FFStatus.ON

  const [user, setUser] = useState<CloudUser | null>(null)
  const [role, setRole] = useState<CloudRole | null>(null)
  const [permissions, setPermissions] = useState<string[]>([])
  const [backupMode, setBackupModeState] = useState<BackupMode>(BackupMode.OFF)
  const [isLoading, setIsLoading] = useState(false)
  const [isInitialized, setIsInitialized] = useState(false)
  const [isPasswordRecoveryActive, setIsPasswordRecoveryActive] =
    useState(false)
  const authProviderRef = useRef<CloudAuthProvider | null>(null)
  const lastSyncedTokenSignatureRef = useRef<string | null>(null)
  const lastBackupSettingsUserIdRef = useRef<string | null>(null)
  const clearBackendAuthInFlightRef = useRef(false)
  const syncInFlightRef = useRef(false)
  const pendingSyncRef = useRef<CloudSession | null | undefined>(undefined)
  const handleAuthStateChangeRef = useRef<
    ((session: CloudSession | null) => Promise<void>) | null
  >(null)
  const isExplicitSignOutRef = useRef(false)
  const recoveryAttemptedRef = useRef(false)

  const getProvider = useCallback(() => {
    if (!authProviderRef.current) {
      authProviderRef.current = createAuthProvider()
    }
    return authProviderRef.current
  }, [])

  const refreshBackupMode = useCallback(async (): Promise<void> => {
    try {
      const settings = await getBackupSettings()
      if (
        settings?.mode === BackupMode.OFF ||
        settings?.mode === BackupMode.AUTO ||
        settings?.mode === BackupMode.MANUAL
      ) {
        setBackupModeState(settings.mode)
      } else {
        setBackupModeState(BackupMode.OFF)
      }
    } catch (error) {
      console.error("Failed to fetch backup settings:", error)
      setBackupModeState(BackupMode.OFF)
    }
  }, [])

  const refreshBackupModeForUser = useCallback(
    async (userId: string): Promise<void> => {
      if (lastBackupSettingsUserIdRef.current === userId) {
        return
      }

      lastBackupSettingsUserIdRef.current = userId
      await refreshBackupMode()
    },
    [refreshBackupMode],
  )

  const syncWithBackend = useCallback(
    async (
      session: CloudSession,
    ): Promise<{
      role: CloudRole | null
      permissions: string[]
    }> => {
      try {
        const response = await cloudAuth({
          token: {
            access_token: session.accessToken,
            refresh_token: session.refreshToken,
            token_type: session.tokenType,
            expires_at: session.expiresAt,
          },
        })
        return { role: response.role, permissions: response.permissions }
      } catch (error) {
        console.error("Failed to sync with backend:", error)
        return { role: null, permissions: [] }
      }
    },
    [],
  )

  const computeTokenSignature = (session: CloudSession): string => {
    return `${session.accessToken}|${session.refreshToken}|${session.tokenType}`
  }

  const clearBackendCloudSession = useCallback(async (): Promise<void> => {
    if (clearBackendAuthInFlightRef.current) {
      return
    }

    clearBackendAuthInFlightRef.current = true
    try {
      await cloudAuth({ token: null })
    } catch (error) {
      console.error("Failed to clear cloud session in backend:", error)
    } finally {
      clearBackendAuthInFlightRef.current = false
    }
  }, [])

  const syncRoleWithBackendIfNeeded = useCallback(
    async (session: CloudSession) => {
      const signature = computeTokenSignature(session)

      if (signature === lastSyncedTokenSignatureRef.current) {
        return
      }

      if (syncInFlightRef.current) {
        pendingSyncRef.current = session
        return
      }

      syncInFlightRef.current = true
      try {
        const { role: backendRole, permissions: backendPermissions } =
          await syncWithBackend(session)
        lastSyncedTokenSignatureRef.current = signature
        setRole(backendRole)
        setPermissions(backendPermissions)
      } finally {
        syncInFlightRef.current = false
        const pending = pendingSyncRef.current
        pendingSyncRef.current = undefined
        if (pending !== undefined) {
          if (pending) {
            void syncRoleWithBackendIfNeeded(pending)
          }
        }
      }
    },
    [syncWithBackend],
  )

  const refreshSessionIfNeeded = useCallback(async (): Promise<void> => {
    try {
      const provider = getProvider()
      const session = await provider.getSession()
      if (session) {
        const now = Math.floor(Date.now() / 1000)
        const expiresAt = session.expiresAt
        const timeUntilExpiry = expiresAt - now
        const shouldRefresh = timeUntilExpiry < 300

        if (shouldRefresh) {
          await provider.refreshSession()
        }
      }
    } catch (error) {
      console.error("Failed to refresh session:", error)
    }
  }, [getProvider])

  const clearLocalCloudState = useCallback(() => {
    setUser(null)
    setRole(null)
    setPermissions([])
    setBackupModeState(BackupMode.OFF)
    lastSyncedTokenSignatureRef.current = null
    lastBackupSettingsUserIdRef.current = null
  }, [])

  const handleAuthStateChange = useCallback(
    async (session: CloudSession | null) => {
      if (session) {
        recoveryAttemptedRef.current = false
        setUser(session.user)
        try {
          await getProvider().setAutoRefreshEnabled(true)
        } catch (error) {
          console.error("Failed to enable auto-refresh:", error)
        }
        await syncRoleWithBackendIfNeeded(session)
        await refreshBackupModeForUser(session.user.id)
      } else if (isExplicitSignOutRef.current) {
        isExplicitSignOutRef.current = false
        await clearBackendCloudSession()
        clearLocalCloudState()
        try {
          await getProvider().setAutoRefreshEnabled(false)
        } catch (error) {
          console.error("Failed to disable auto-refresh:", error)
        }
      } else if (!recoveryAttemptedRef.current) {
        recoveryAttemptedRef.current = true
        try {
          const authData = await getCloudAuthToken()
          if (authData?.token) {
            const provider = getProvider()
            await provider.setSession(
              authData.token.access_token,
              authData.token.refresh_token,
            )
            const recovered = await provider.getSession()
            if (recovered) {
              await handleAuthStateChangeRef.current?.(recovered)
              return
            }
          }
        } catch (error) {
          console.error("Failed to recover cloud session:", error)
        }
        await clearBackendCloudSession()
        clearLocalCloudState()
        try {
          await getProvider().setAutoRefreshEnabled(false)
        } catch (error) {
          console.error("Failed to disable auto-refresh:", error)
        }
      } else {
        clearLocalCloudState()
        try {
          await getProvider().setAutoRefreshEnabled(false)
        } catch (error) {
          console.error("Failed to disable auto-refresh:", error)
        }
      }
    },
    [
      clearBackendCloudSession,
      clearLocalCloudState,
      getProvider,
      refreshBackupModeForUser,
      syncRoleWithBackendIfNeeded,
    ],
  )

  // Keep ref updated synchronously - not in an effect
  handleAuthStateChangeRef.current = handleAuthStateChange

  // Refresh session when app becomes visible after being hidden
  useEffect(() => {
    if (!isAuthenticated || !isCloudEnabled || !user) {
      return
    }

    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        void refreshSessionIfNeeded()
      }
    }

    document.addEventListener("visibilitychange", handleVisibilityChange)

    return () => {
      document.removeEventListener("visibilitychange", handleVisibilityChange)
    }
  }, [isAuthenticated, isCloudEnabled, user, refreshSessionIfNeeded])

  // Periodic session refresh check (every 5 minutes)
  useEffect(() => {
    if (!isAuthenticated || !isCloudEnabled || !user) {
      return
    }

    const intervalId = setInterval(
      () => {
        void refreshSessionIfNeeded()
      },
      5 * 60 * 1000,
    )

    return () => {
      clearInterval(intervalId)
    }
  }, [isAuthenticated, isCloudEnabled, user, refreshSessionIfNeeded])

  // When cloud feature is disabled, mark as initialized immediately.
  // Feature flags start empty until the auth status loads, so only trust
  // `isCloudEnabled === false` once the user is authenticated (flags are set
  // in the same commit as authentication). Before that, keep isInitialized
  // false so consumers don't act on a transient OFF backup mode.
  useEffect(() => {
    if (!isAuthenticated) {
      setIsInitialized(false)
      return
    }
    if (!isCloudEnabled) {
      setUser(null)
      setRole(null)
      setPermissions([])
      setBackupModeState(BackupMode.OFF)
      lastBackupSettingsUserIdRef.current = null
      setIsInitialized(true)
    }
  }, [isAuthenticated, isCloudEnabled])

  useEffect(() => {
    if (!isAuthenticated || !isCloudEnabled) {
      return
    }

    let cancelled = false
    let unsubscribe: (() => void) | undefined

    const initialize = async () => {
      try {
        const provider = getProvider()
        await provider.initialize()
        if (cancelled) return

        let session = await provider.getSession()
        if (cancelled) return

        try {
          const authData = await getCloudAuthToken()
          if (cancelled) return

          if (authData?.token) {
            try {
              await provider.setSession(
                authData.token.access_token,
                authData.token.refresh_token,
              )
              if (cancelled) return
              session = await provider.getSession()
              if (cancelled) return
              setRole(authData.role)
              setPermissions(authData.permissions)
            } catch (error) {
              console.error("Failed to set local session from backend:", error)
            }
          } else {
            if (session) {
              try {
                await provider.clearLocalSession()
              } catch (error) {
                console.error(
                  "Failed to clear local cloud session after backend returned null:",
                  error,
                )
              }
              session = null
            }
          }
        } catch (error) {
          console.error("Failed to fetch cloud session from backend:", error)
        }

        if (cancelled) return

        await provider.setAutoRefreshEnabled(!!session)
        if (cancelled) return

        if (session) {
          await handleAuthStateChangeRef.current?.(session)
        } else {
          setUser(null)
          setRole(null)
          setPermissions([])
          setBackupModeState(BackupMode.OFF)
          lastBackupSettingsUserIdRef.current = null
        }

        if (cancelled) return

        unsubscribe = provider.onAuthStateChange(changedSession => {
          void handleAuthStateChangeRef.current?.(changedSession)
        })
      } catch (error) {
        console.error("Failed to initialize cloud auth:", error)
      } finally {
        if (!cancelled) {
          setIsInitialized(true)
        }
      }
    }

    initialize()

    return () => {
      cancelled = true
      unsubscribe?.()
    }
  }, [getProvider, isAuthenticated, isCloudEnabled, syncWithBackend])

  useEffect(() => {
    try {
      const provider = getProvider()
      const unsubscribe = provider.onAuthEvent(event => {
        if (event === "PASSWORD_RECOVERY") {
          setIsPasswordRecoveryActive(true)
          navigate("/settings?tab=cloud")
        }
      })

      return () => {
        unsubscribe?.()
      }
    } catch {
      return
    }
  }, [getProvider, navigate])

  const clearPasswordRecovery = useCallback(() => {
    setIsPasswordRecoveryActive(false)
  }, [])

  const updatePassword = useCallback(
    async (password: string) => {
      setIsLoading(true)
      try {
        const provider = getProvider()
        await provider.updatePassword(password)
      } finally {
        setIsLoading(false)
      }
    },
    [getProvider],
  )

  const signInWithEmail = useCallback(
    async (email: string, password: string) => {
      setIsLoading(true)
      try {
        const provider = getProvider()
        await provider.signInWithEmail(email, password)
      } finally {
        setIsLoading(false)
      }
    },
    [getProvider],
  )

  const signUpWithEmail = useCallback(
    async (email: string, password: string) => {
      setIsLoading(true)
      try {
        const provider = getProvider()
        return await provider.signUpWithEmail(email, password)
      } finally {
        setIsLoading(false)
      }
    },
    [getProvider],
  )

  const requestPasswordReset = useCallback(
    async (email: string) => {
      setIsLoading(true)
      try {
        const provider = getProvider()
        await provider.requestPasswordReset(email)
      } finally {
        setIsLoading(false)
      }
    },
    [getProvider],
  )

  const signOut = useCallback(async () => {
    setIsLoading(true)
    try {
      isExplicitSignOutRef.current = true
      const provider = getProvider()
      try {
        await provider.signOut()
      } catch (error) {
        isExplicitSignOutRef.current = false
        throw error
      }
      resetBackupStatusCache()
    } finally {
      setIsLoading(false)
    }
  }, [getProvider])

  const setBackupMode = useCallback(
    (mode: BackupMode) => {
      if (!user) {
        return
      }

      setBackupModeState(previousMode => {
        if (previousMode === mode) {
          return previousMode
        }

        void (async () => {
          try {
            await updateBackupSettings({ mode })
          } catch (error) {
            console.error("Failed to persist backup settings:", error)
            setBackupModeState(previousMode)
          }
        })()

        return mode
      })
    },
    [user],
  )

  return (
    <CloudContext.Provider
      value={{
        user,
        role,
        permissions,
        backupMode,
        setBackupMode,
        isLoading,
        isInitialized,
        isPasswordRecoveryActive,
        clearPasswordRecovery,
        signInWithEmail,
        signUpWithEmail,
        requestPasswordReset,
        updatePassword,
        signOut,
      }}
    >
      {children}
    </CloudContext.Provider>
  )
}

export function useCloud(): CloudContextType {
  const context = useContext(CloudContext)
  if (context === undefined) {
    throw new Error("useCloud must be used within a CloudProvider")
  }
  return context
}
