import type React from "react"
import { useState, useCallback, useRef, useEffect } from "react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { LoadingSpinner } from "@/components/ui/LoadingSpinner"
import { motion, AnimatePresence } from "framer-motion"
import {
  ArrowLeft,
  Download,
  Mail,
  AlertCircle,
  ShieldAlert,
} from "lucide-react"
import { useI18n } from "@/i18n"
import { cn } from "@/lib/utils"
import { SupabaseAuthProvider } from "@/services/cloud/supabaseProvider"
import type { CloudSession } from "@/services/cloud/types"
import {
  getBackupsInfoWithCloudAuth,
  importBackup,
  cloudAuth,
} from "@/services/api"
import { useAuth } from "@/context/AuthContext"
import { useTheme } from "@/context/ThemeContext"
import {
  authInputClass,
  authPasswordValueClass,
  isLightTheme,
} from "@/components/auth/authStyles"
import { ErrorReportingToggle } from "@/components/telemetry/ErrorReportingToggle"
import type { FullBackupsInfo, BackupFileType } from "@/types"

type RestoreStep = "login" | "checking" | "credentials" | "importing" | "error"

interface CloudRestoreProps {
  onBack: () => void
  onRestoreComplete: () => void
  pendingUsername?: string
}

export function CloudRestore({
  onBack,
  onRestoreComplete,
  pendingUsername,
}: CloudRestoreProps) {
  const { t } = useI18n()
  const { guestSignup, signup, logout } = useAuth()
  const { theme } = useTheme()

  const isLight = isLightTheme(theme)
  const inputClass = authInputClass(isLight)

  const [step, setStep] = useState<RestoreStep>("login")
  const [email, setEmail] = useState("")
  const [cloudPassword, setCloudPassword] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [loadingMethod, setLoadingMethod] = useState<
    "email" | "restore" | null
  >(null)
  const [cloudSession, setCloudSession] = useState<CloudSession | null>(null)
  const [backups, setBackups] = useState<FullBackupsInfo | null>(null)
  const [restoreUsername, setRestoreUsername] = useState(pendingUsername || "")
  const [encryptionKey, setEncryptionKey] = useState("")
  const [guestCreated, setGuestCreated] = useState(false)

  const providerRef = useRef<SupabaseAuthProvider | null>(null)

  const getProvider = useCallback(() => {
    if (!providerRef.current) {
      providerRef.current = new SupabaseAuthProvider()
    }
    return providerRef.current
  }, [])

  useEffect(() => {
    const init = async () => {
      const provider = getProvider()
      await provider.initialize()
    }
    init()
  }, [getProvider])

  const handleCloudLoginSuccess = useCallback(
    async (session: CloudSession) => {
      setCloudSession(session)
      setStep("checking")
      setError(null)

      try {
        const backupsInfo = await getBackupsInfoWithCloudAuth(
          session.accessToken,
        )

        const hasBackups = Object.values(backupsInfo.pieces).some(
          piece => piece.remote !== null,
        )

        if (!hasBackups) {
          setError(t.login.cloudRestore.noBackupsFound)
          setStep("login")
          setLoadingMethod(null)
          return
        }

        setBackups(backupsInfo)
        setStep("credentials")
      } catch (err) {
        console.error("Failed to check backups:", err)
        setError(t.login.cloudRestore.backupCheckFailed)
        setStep("login")
      } finally {
        setLoadingMethod(null)
      }
    },
    [t],
  )

  const handleEmailSignIn = async (e: React.FormEvent) => {
    e.preventDefault()
    setError(null)
    setLoadingMethod("email")

    try {
      const provider = getProvider()
      await provider.signInWithEmail(email, cloudPassword)
      const session = await provider.getSession()
      if (!session) {
        setError(t.settings.cloud.loginError)
        return
      }
      await handleCloudLoginSuccess(session)
    } catch (err: unknown) {
      console.error("Cloud sign-in error:", err)
      const maybeMessage =
        typeof err === "object" && err && "message" in err
          ? (err as { message?: string }).message
          : null
      if (
        maybeMessage?.includes("Invalid login") ||
        maybeMessage?.includes("invalid_credentials")
      ) {
        setError(t.settings.cloud.loginErrorInvalidCredentials)
      } else if (maybeMessage?.includes("Email not confirmed")) {
        setError(t.settings.cloud.loginErrorEmailNotConfirmed)
      } else {
        setError(t.settings.cloud.loginError)
      }
    } finally {
      setLoadingMethod(null)
    }
  }

  const handleRestore = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!cloudSession || !backups) return

    setError(null)
    setLoadingMethod("restore")
    setStep("importing")

    try {
      if (!guestCreated) {
        const success = await guestSignup(restoreUsername)
        if (!success) {
          setError(t.login.cloudRestore.guestCreationFailed)
          setStep("credentials")
          setLoadingMethod(null)
          return
        }
        setGuestCreated(true)
      }

      await cloudAuth({
        token: {
          access_token: cloudSession.accessToken,
          refresh_token: cloudSession.refreshToken,
          token_type: cloudSession.tokenType,
          expires_at: cloudSession.expiresAt,
        },
      })

      const backupTypes: BackupFileType[] = Object.keys(backups.pieces).filter(
        key => backups.pieces[key as BackupFileType]?.remote !== null,
      ) as BackupFileType[]

      await importBackup({
        types: backupTypes,
        password: encryptionKey,
        force: true,
        initialize: true,
      })

      await logout()

      const success2 = await signup(restoreUsername, encryptionKey)
      if (!success2) {
        setError(t.login.cloudRestore.loginAfterRestoreFailed)
        setStep("credentials")
        setLoadingMethod(null)
        return
      }

      onRestoreComplete()
    } catch (err: unknown) {
      console.error("Restore failed:", err)
      const maybeCode =
        typeof err === "object" && err && "code" in err
          ? (err as { code?: string }).code
          : null

      if (maybeCode === "INVALID_BACKUP_CREDENTIALS") {
        setError(t.login.cloudRestore.invalidEncryptionKey)
      } else if (maybeCode === "TOO_MANY_REQUESTS") {
        setError(t.login.cloudRestore.tooManyRequests)
      } else {
        setError(t.login.cloudRestore.restoreFailed)
      }
      setStep("credentials")
    } finally {
      setLoadingMethod(null)
    }
  }

  const handleBack = async () => {
    if (step === "importing") return

    if (guestCreated) {
      try {
        await logout()
      } catch {
        // ignore
      }
      setGuestCreated(false)
    }

    if (cloudSession) {
      try {
        const provider = getProvider()
        await provider.signOut()
      } catch {
        // ignore
      }
      setCloudSession(null)
    }

    setStep("login")
    setError(null)
    setBackups(null)
    setRestoreUsername("")
    setEncryptionKey("")
    onBack()
  }

  const isImporting = step === "importing"

  if (step === "checking") {
    return (
      <div className="flex flex-col items-center justify-center gap-4 py-8">
        <LoadingSpinner size="lg" />
        <p className="text-sm text-muted-foreground">
          {t.login.cloudRestore.checkingBackups}
        </p>
      </div>
    )
  }

  if (step === "importing") {
    return (
      <div className="flex flex-col items-center justify-center gap-4 py-8">
        <LoadingSpinner size="lg" />
        <p className="text-base font-medium">
          {t.login.cloudRestore.importing}
        </p>
        <div className="flex flex-col items-center gap-1 text-amber-500">
          <ShieldAlert className="h-4 w-4" />
          <p className="text-sm text-center">
            {t.login.cloudRestore.dontCloseApp}
          </p>
        </div>
      </div>
    )
  }

  if (step === "credentials") {
    return (
      <AnimatePresence mode="wait">
        <motion.div
          key="credentials"
          initial={{ opacity: 0, x: 20 }}
          animate={{ opacity: 1, x: 0 }}
          exit={{ opacity: 0, x: -20 }}
          transition={{ duration: 0.2 }}
          className="space-y-6 mx-auto max-w-md"
        >
          <div className="text-center space-y-2">
            <Download className="h-8 w-8 mx-auto text-primary" />
            <p className="text-base font-medium">
              {t.login.cloudRestore.backupFound}
            </p>
          </div>

          <form onSubmit={handleRestore} className="space-y-4">
            <input
              id="restoreUsername"
              type="text"
              value={restoreUsername}
              onChange={e => setRestoreUsername(e.target.value)}
              placeholder={t.login.namePlaceholder}
              required
              autoCapitalize="off"
              disabled={!!loadingMethod || !!pendingUsername}
              className={inputClass}
            />

            <div className="space-y-2">
              <input
                id="encryptionKey"
                type="password"
                value={encryptionKey}
                onChange={e => setEncryptionKey(e.target.value)}
                placeholder={t.login.cloudRestore.encryptionKeyPlaceholder}
                required
                disabled={!!loadingMethod}
                className={cn(inputClass, authPasswordValueClass)}
              />
              <p className="text-xs text-muted-foreground text-center">
                {t.login.cloudRestore.encryptionKeyHint}
              </p>
            </div>

            {error && (
              <motion.div
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                className="text-sm text-destructive text-center flex items-center justify-center gap-2"
              >
                <AlertCircle className="h-4 w-4 shrink-0" />
                {error}
              </motion.div>
            )}

            <div className="flex items-center gap-2">
              <Button
                type="submit"
                variant="ghost"
                className={cn(
                  "flex-1 min-w-0 text-base py-6 font-bold tracking-wide bg-transparent shadow-none hover:bg-transparent dark:bg-transparent dark:hover:bg-transparent disabled:opacity-40",
                  isLight
                    ? "text-black hover:text-black"
                    : "text-white hover:text-white dark:text-white dark:hover:text-white",
                )}
                disabled={!!loadingMethod || !restoreUsername || !encryptionKey}
              >
                {loadingMethod === "restore" ? (
                  <>
                    <LoadingSpinner size="sm" className="mr-2" />
                    {t.login.cloudRestore.importing}
                  </>
                ) : (
                  <>
                    <Download className="h-4 w-4 mr-2 shrink-0" />
                    {t.login.cloudRestore.restoreButton}
                  </>
                )}
              </Button>
              <ErrorReportingToggle className="shrink-0" />
            </div>
          </form>

          <button
            type="button"
            onClick={handleBack}
            disabled={isImporting}
            className="w-full text-sm text-muted-foreground hover:text-foreground transition-colors flex items-center justify-center gap-1"
          >
            <ArrowLeft className="h-3.5 w-3.5" />
            {t.login.cloudRestore.backToSignup}
          </button>
        </motion.div>
      </AnimatePresence>
    )
  }

  return (
    <AnimatePresence mode="wait">
      <motion.div
        key="cloud-login"
        initial={{ opacity: 0, x: 20 }}
        animate={{ opacity: 1, x: 0 }}
        exit={{ opacity: 0, x: -20 }}
        transition={{ duration: 0.2 }}
        className="space-y-6 mx-auto max-w-md"
      >
        <form onSubmit={handleEmailSignIn} className="space-y-4">
          <Input
            type="email"
            placeholder={t.settings.cloud.emailPlaceholder}
            value={email}
            onChange={e => setEmail(e.target.value)}
            required
            disabled={!!loadingMethod}
          />
          <Input
            type="password"
            placeholder={t.settings.cloud.passwordPlaceholder}
            value={cloudPassword}
            onChange={e => setCloudPassword(e.target.value)}
            required
            disabled={!!loadingMethod}
            minLength={6}
          />

          {error && (
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              className="text-sm text-destructive text-center flex items-center justify-center gap-2"
            >
              <AlertCircle className="h-4 w-4 shrink-0" />
              {error}
            </motion.div>
          )}

          <Button
            type="submit"
            disabled={!!loadingMethod}
            className="w-full"
            size="lg"
          >
            {loadingMethod === "email" ? (
              <>
                <LoadingSpinner size="sm" className="mr-2" />
                {t.settings.cloud.loggingIn}
              </>
            ) : (
              <>
                <Mail className="mr-2 h-4 w-4" />
                {t.settings.cloud.signInWithEmail}
              </>
            )}
          </Button>
        </form>

        <button
          type="button"
          onClick={handleBack}
          className="w-full text-sm text-muted-foreground hover:text-foreground transition-colors flex items-center justify-center gap-1"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
          {t.login.cloudRestore.backToSignup}
        </button>
      </motion.div>
    </AnimatePresence>
  )
}
