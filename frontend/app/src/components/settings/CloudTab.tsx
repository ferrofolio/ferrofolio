import { useEffect, useState } from "react"
import { motion } from "framer-motion"
import {
  HardDrive,
  LogOut,
  Mail,
  MoreVertical,
  Trash2,
  User,
} from "lucide-react"
import { Button } from "@/components/ui/Button"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import { LoadingSpinner } from "@/components/ui/LoadingSpinner"
import { Badge } from "@/components/ui/Badge"
import { useI18n } from "@/i18n"
import { useCloud } from "@/context/CloudContext"
import { CloudRole } from "@/types"
import { useBackupStatus } from "@/hooks/useBackupStatus"
import { BackupStatusContent } from "@/components/backup/BackupStatusContent"
import { ConfirmationDialog } from "@/components/ui/ConfirmationDialog"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/Popover"

export function CloudTab() {
  const { t } = useI18n()
  const {
    user,
    role,
    permissions,
    isLoading,
    isInitialized,
    signInWithEmail,
    signUpWithEmail,
    requestPasswordReset,
    isPasswordRecoveryActive,
    clearPasswordRecovery,
    updatePassword,
    signOut,
  } = useCloud()

  const backupStatus = useBackupStatus({ isActive: true })

  const [email, setEmail] = useState("")
  const [password, setPassword] = useState("")
  const [confirmPassword, setConfirmPassword] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [success, setSuccess] = useState<React.ReactNode | null>(null)
  const [authMode, setAuthMode] = useState<"signIn" | "signUp">("signIn")
  const [newPassword, setNewPassword] = useState("")
  const [confirmNewPassword, setConfirmNewPassword] = useState("")
  const [isLogoutConfirmationOpen, setIsLogoutConfirmationOpen] =
    useState(false)
  const [activeAction, setActiveAction] = useState<
    | null
    | "emailSignIn"
    | "emailSignUp"
    | "passwordResetRequest"
    | "passwordUpdate"
    | "signOut"
  >(null)

  const isSignedIn = !!user
  const canSeeBackup = permissions.includes("backup.info")

  useEffect(() => {
    if (isPasswordRecoveryActive) {
      setError(null)
      setSuccess(null)
    }
  }, [isPasswordRecoveryActive])

  const TERMS_URL = "https://finanze.me/terms"
  const PRIVACY_URL = "https://finanze.me/privacy"

  const openExternalUrl = (url: string) => {
    try {
      window.open(url, "_blank")
    } catch {
      // ignore
    }
  }

  const handleSignOut = async () => {
    setError(null)
    setSuccess(null)
    setActiveAction("signOut")
    try {
      await signOut()
      setIsLogoutConfirmationOpen(false)
    } finally {
      setActiveAction(null)
    }
  }

  const handleEmailAuth = async (e: React.FormEvent) => {
    e.preventDefault()
    setError(null)
    setSuccess(null)

    setActiveAction(authMode === "signIn" ? "emailSignIn" : "emailSignUp")
    try {
      if (authMode === "signIn") {
        await signInWithEmail(email, password)
        setEmail("")
        setPassword("")
        return
      }

      if (!/(?=.*[a-zA-Z])(?=.*\d).{8,}/.test(password)) {
        setError(t.settings.cloud.passwordValidationError)
        setActiveAction(null)
        return
      }

      if (password !== confirmPassword) {
        setError(t.settings.cloud.passwordMismatch)
        setActiveAction(null)
        return
      }

      const result = await signUpWithEmail(email, password)
      setPassword("")
      setConfirmPassword("")

      if (result.status === "EMAIL_ALREADY_REGISTERED") {
        setError(t.settings.cloud.signUpErrors.email_exists)
        return
      }

      if (result.status === "PENDING_EMAIL_CONFIRMATION") {
        const template = t.settings.cloud.signUpSuccessCheckEmail
        const parts = template.split("{email}")
        setSuccess(
          <>
            {parts[0]}
            <strong className="font-semibold">{result.email}</strong>
            {parts.slice(1).join("{email}")}
          </>,
        )
      } else {
        setEmail("")
        setSuccess(t.settings.cloud.signUpSuccess)
      }
    } catch (err: unknown) {
      console.error("Cloud sign-in error:", err)

      const maybeStatus =
        typeof err === "object" && err && "status" in err
          ? (err as { status?: unknown }).status
          : undefined

      const maybeCode =
        typeof err === "object" && err && "code" in err
          ? (err as { code?: unknown }).code
          : undefined

      const maybeMessage =
        typeof err === "object" && err && "message" in err
          ? (err as { message?: unknown }).message
          : undefined

      const code = typeof maybeCode === "string" ? maybeCode : null
      const message = typeof maybeMessage === "string" ? maybeMessage : null
      const status = typeof maybeStatus === "number" ? maybeStatus : null

      if (status === 429) {
        setError(t.settings.cloud.tooManyRequests)
        return
      }

      if (authMode === "signUp") {
        const fallbackCode = code ?? "unknown"
        const translatedError =
          t.settings.cloud.signUpErrors[
            fallbackCode as keyof typeof t.settings.cloud.signUpErrors
          ] ?? t.settings.cloud.signUpErrors.unknown

        setError(translatedError.replace("{error}", fallbackCode))
        return
      }

      if (
        code === "invalid_credentials" ||
        message === "Invalid login credentials"
      ) {
        setError(t.settings.cloud.loginErrorInvalidCredentials)
        return
      }

      if (
        code === "email_not_confirmed" ||
        message?.toLowerCase().includes("not confirmed")
      ) {
        setError(t.settings.cloud.loginErrorEmailNotConfirmed)
        return
      }

      setError(t.settings.cloud.loginError)
    } finally {
      setActiveAction(null)
    }
  }

  const handleForgotPassword = async () => {
    setError(null)
    setSuccess(null)

    const normalizedEmail = email.trim()
    if (!normalizedEmail) {
      setError(t.settings.cloud.passwordResetEmailRequired)
      return
    }

    setActiveAction("passwordResetRequest")
    try {
      await requestPasswordReset(normalizedEmail)
      const template = t.settings.cloud.passwordResetEmailSent
      const parts = template.split("{email}")
      setSuccess(
        <>
          {parts[0]}
          <strong className="font-semibold">{normalizedEmail}</strong>
          {parts.slice(1).join("{email}")}
        </>,
      )
    } catch (err: unknown) {
      console.error("Password reset request error:", err)

      const maybeStatus =
        typeof err === "object" && err && "status" in err
          ? (err as { status?: unknown }).status
          : undefined

      const status = typeof maybeStatus === "number" ? maybeStatus : null
      if (status === 429) {
        setError(t.settings.cloud.tooManyRequests)
        return
      }

      setError(t.settings.cloud.passwordResetError)
    } finally {
      setActiveAction(null)
    }
  }

  const handleUpdatePassword = async (e: React.FormEvent) => {
    e.preventDefault()
    setError(null)
    setSuccess(null)

    if (!newPassword || !confirmNewPassword) {
      setError(t.settings.cloud.passwordUpdateError)
      return
    }

    if (newPassword !== confirmNewPassword) {
      setError(t.settings.cloud.passwordMismatch)
      return
    }

    if (!/(?=.*[a-zA-Z])(?=.*\d).{8,}/.test(newPassword)) {
      setError(t.settings.cloud.passwordValidationError)
      return
    }

    setActiveAction("passwordUpdate")
    try {
      await updatePassword(newPassword)
      setNewPassword("")
      setConfirmNewPassword("")
      setSuccess(t.settings.cloud.passwordUpdateSuccess)
      clearPasswordRecovery()
    } catch (err: unknown) {
      console.error("Password update error:", err)

      const maybeStatus =
        typeof err === "object" && err && "status" in err
          ? (err as { status?: unknown }).status
          : undefined

      const maybeCode =
        typeof err === "object" && err && "code" in err
          ? (err as { code?: unknown }).code
          : undefined

      const status = typeof maybeStatus === "number" ? maybeStatus : null
      const code = typeof maybeCode === "string" ? maybeCode : null

      if (status === 429) {
        setError(t.settings.cloud.tooManyRequests)
        return
      }

      if (code === "weak_password") {
        setError(t.settings.cloud.signUpErrors.weak_password)
        return
      }

      if (code === "same_password") {
        setError(t.settings.cloud.signUpErrors.same_password)
        return
      }

      setError(t.settings.cloud.passwordUpdateError)
    } finally {
      setActiveAction(null)
    }
  }

  if (!isInitialized) {
    return (
      <div className="flex justify-center items-center h-64">
        <LoadingSpinner size="lg" />
      </div>
    )
  }

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.3 }}
      className="space-y-4"
    >
      <Card
        className={
          role === CloudRole.PLUS
            ? "-mx-6 md:mx-0 rounded-none md:rounded-lg border-x-0 md:border-x border-amber-400/60 shadow-[0_0_0_1px_rgba(251,191,36,0.18)]"
            : "-mx-6 md:mx-0 rounded-none md:rounded-lg border-x-0 md:border-x"
        }
      >
        <CardHeader>
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <User className="h-5 w-5 text-primary" />
              <CardTitle>{t.settings.cloud.accountTitle}</CardTitle>
            </div>
            {role === CloudRole.PLUS ? (
              <Badge className="bg-gradient-to-r from-amber-500 to-orange-500 text-white border-0">
                {t.settings.cloud.roles[role]}
              </Badge>
            ) : role === CloudRole.BASIC ? (
              <Badge variant="outline">{t.settings.cloud.roles[role]}</Badge>
            ) : null}
          </div>
          <CardDescription>{t.settings.cloud.description}</CardDescription>
        </CardHeader>
        <CardContent>
          {isPasswordRecoveryActive ? (
            <div className="space-y-4 mx-auto max-w-md">
              <div className="space-y-1 text-center">
                <p className="text-base font-medium">
                  {t.settings.cloud.passwordResetTitle}
                </p>
                <p className="text-sm text-muted-foreground">
                  {t.settings.cloud.passwordResetDescription}
                </p>
              </div>

              <form onSubmit={handleUpdatePassword} className="space-y-4">
                <Input
                  type="password"
                  placeholder={t.settings.cloud.newPasswordPlaceholder}
                  value={newPassword}
                  onChange={e => setNewPassword(e.target.value)}
                  required
                  disabled={isLoading}
                  minLength={6}
                />
                <Input
                  type="password"
                  placeholder={t.settings.cloud.confirmNewPasswordPlaceholder}
                  value={confirmNewPassword}
                  onChange={e => setConfirmNewPassword(e.target.value)}
                  required
                  disabled={isLoading}
                  minLength={6}
                />

                {success && (
                  <p className="text-sm text-primary text-center">{success}</p>
                )}
                {error && (
                  <p className="text-sm text-destructive text-center">
                    {error}
                  </p>
                )}

                <Button
                  type="submit"
                  disabled={isLoading}
                  className="w-full"
                  size="lg"
                >
                  {isLoading ? (
                    <>
                      <LoadingSpinner size="sm" className="mr-2" />
                      {t.common.saving}
                    </>
                  ) : (
                    t.settings.cloud.updatePassword
                  )}
                </Button>

                <Button
                  type="button"
                  variant="outline"
                  disabled={isLoading}
                  className="w-full"
                  onClick={() => {
                    setError(null)
                    setSuccess(null)
                    setNewPassword("")
                    setConfirmNewPassword("")
                    clearPasswordRecovery()
                  }}
                >
                  {t.settings.cloud.cancelPasswordReset}
                </Button>
              </form>
            </div>
          ) : isSignedIn ? (
            <div className="space-y-6">
              <div className="-mx-6">
                <div className="flex w-full flex-col gap-4 border-y border-border/50 bg-muted/20 p-4 dark:bg-muted/10 sm:flex-row sm:items-center sm:justify-between">
                  <div className="space-y-1">
                    <p className="text-sm text-muted-foreground">
                      {t.settings.cloud.signedInAs}
                    </p>
                    <p className="font-medium">{user.email}</p>
                  </div>
                  <div className="flex items-center gap-2 self-end sm:self-auto">
                    <Popover>
                      <PopoverTrigger asChild>
                        <Button variant="ghost" size="icon" className="h-8 w-8">
                          <MoreVertical className="h-4 w-4" />
                        </Button>
                      </PopoverTrigger>
                      <PopoverContent align="end" className="w-auto p-1">
                        <button
                          type="button"
                          className="flex w-full items-center gap-2 rounded-sm px-3 py-2 text-sm text-destructive hover:bg-destructive/10 transition-colors"
                          onClick={() => {
                            const subject = encodeURIComponent(
                              t.settings.cloud.deleteAccountEmailSubject,
                            )
                            const body = encodeURIComponent(
                              t.settings.cloud.deleteAccountEmailBody.replace(
                                "{email}",
                                user.email,
                              ),
                            )
                            openExternalUrl(
                              `mailto:contact@finanze.me?subject=${subject}&body=${body}`,
                            )
                          }}
                        >
                          <Trash2 className="h-4 w-4" />
                          {t.settings.cloud.requestAccountDeletion}
                        </button>
                      </PopoverContent>
                    </Popover>
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => setIsLogoutConfirmationOpen(true)}
                      disabled={isLoading}
                      aria-label={t.settings.cloud.logout}
                    >
                      {isLoading && activeAction === "signOut" ? (
                        <LoadingSpinner size="sm" />
                      ) : (
                        <LogOut className="h-4 w-4" />
                      )}
                    </Button>
                  </div>
                </div>
              </div>
            </div>
          ) : (
            <div className="space-y-6 mx-auto max-w-md">
              <form onSubmit={handleEmailAuth} className="space-y-4">
                <Input
                  type="email"
                  placeholder={t.settings.cloud.emailPlaceholder}
                  value={email}
                  onChange={e => setEmail(e.target.value)}
                  required
                  disabled={isLoading}
                />
                <Input
                  type="password"
                  placeholder={t.settings.cloud.passwordPlaceholder}
                  value={password}
                  onChange={e => setPassword(e.target.value)}
                  required
                  disabled={isLoading}
                  minLength={authMode === "signUp" ? 8 : 6}
                />
                {authMode === "signUp" && password.length >= 4 && (
                  <Input
                    type="password"
                    placeholder={t.settings.cloud.confirmPasswordPlaceholder}
                    value={confirmPassword}
                    onChange={e => setConfirmPassword(e.target.value)}
                    required
                    disabled={isLoading}
                    minLength={8}
                  />
                )}
                {success && (
                  <p className="text-sm text-primary text-center">{success}</p>
                )}
                {error && (
                  <p className="text-sm text-destructive text-center">
                    {error}
                  </p>
                )}
                <Button
                  type="submit"
                  disabled={isLoading}
                  className="w-full"
                  size="lg"
                >
                  {isLoading &&
                  (activeAction === "emailSignIn" ||
                    activeAction === "emailSignUp") ? (
                    <>
                      <LoadingSpinner size="sm" className="mr-2" />
                      {authMode === "signIn"
                        ? t.settings.cloud.loggingIn
                        : t.settings.cloud.signingUp}
                    </>
                  ) : (
                    <>
                      <Mail className="mr-2 h-4 w-4" />
                      {authMode === "signIn"
                        ? t.settings.cloud.signInWithEmail
                        : t.settings.cloud.signUp}
                    </>
                  )}
                </Button>
              </form>

              <div className="flex items-center justify-between text-xs text-muted-foreground">
                {authMode === "signIn" ? (
                  <button
                    type="button"
                    disabled={isLoading}
                    className="text-muted-foreground"
                    onClick={handleForgotPassword}
                  >
                    {isLoading && activeAction === "passwordResetRequest" ? (
                      <span className="inline-flex items-center justify-center">
                        <LoadingSpinner size="sm" className="mr-2" />
                        {t.settings.cloud.sendingPasswordResetEmail}
                      </span>
                    ) : (
                      t.settings.cloud.forgotPassword
                    )}
                  </button>
                ) : (
                  <span />
                )}
                <button
                  type="button"
                  disabled={isLoading}
                  className="text-muted-foreground"
                  onClick={() => {
                    setError(null)
                    setSuccess(null)
                    setConfirmPassword("")
                    setAuthMode(prev =>
                      prev === "signIn" ? "signUp" : "signIn",
                    )
                  }}
                >
                  {authMode === "signIn" ? (
                    <>
                      {t.settings.cloud.noAccountPrefix}{" "}
                      <span className="font-semibold text-foreground">
                        {t.settings.cloud.noAccountAction}
                      </span>
                    </>
                  ) : (
                    <>
                      {t.settings.cloud.alreadyHaveAccountPrefix}{" "}
                      <span className="font-semibold text-foreground">
                        {t.settings.cloud.alreadyHaveAccountAction}
                      </span>
                    </>
                  )}
                </button>
              </div>

              <p className="text-xs leading-tight text-muted-foreground text-center">
                {t.settings.cloud.legalNoticePrefix}{" "}
                <a
                  href={TERMS_URL}
                  target="_blank"
                  rel="noreferrer noopener"
                  className="underline underline-offset-2 hover:text-foreground"
                  onClick={e => {
                    e.preventDefault()
                    openExternalUrl(TERMS_URL)
                  }}
                >
                  {t.settings.cloud.termsOfService}
                </a>{" "}
                {t.settings.cloud.legalNoticeAnd}{" "}
                <a
                  href={PRIVACY_URL}
                  target="_blank"
                  rel="noreferrer noopener"
                  className="underline underline-offset-2 hover:text-foreground"
                  onClick={e => {
                    e.preventDefault()
                    openExternalUrl(PRIVACY_URL)
                  }}
                >
                  {t.settings.cloud.privacyPolicy}
                </a>
                .
              </p>
            </div>
          )}
        </CardContent>
      </Card>

      {isSignedIn && canSeeBackup && (
        <Card className="-mx-6 md:mx-0 rounded-none md:rounded-lg border-x-0 md:border-x">
          <CardHeader>
            <div className="flex items-center gap-2">
              <HardDrive className="h-5 w-5 text-primary" />
              <CardTitle>{t.settings.backup.enableLabel}</CardTitle>
            </div>
            <CardDescription>
              {t.settings.backup.enableDescription}
            </CardDescription>
          </CardHeader>
          <CardContent>
            <BackupStatusContent
              backups={backupStatus.backups}
              backupEnabled={backupStatus.backupEnabled}
              backupMode={backupStatus.backupMode}
              setBackupMode={backupStatus.setBackupMode}
              isManualMode={backupStatus.isManualMode}
              isLoading={backupStatus.isLoading}
              isUploading={backupStatus.isUploading}
              isImporting={backupStatus.isImporting}
              isSyncing={backupStatus.isSyncing}
              isSyncCooldownActive={backupStatus.isSyncCooldownActive}
              isConflict={backupStatus.isConflict}
              conflictImportTypes={backupStatus.conflictImportTypes}
              conflictUploadTypes={backupStatus.conflictUploadTypes}
              hasCredentialsMismatch={backupStatus.hasCredentialsMismatch}
              baseActionsDisabled={backupStatus.baseActionsDisabled}
              overallStatus={backupStatus.overallStatus}
              lastBackupDate={backupStatus.lastBackupDate}
              feedbackMessage={backupStatus.feedbackMessage}
              canCreateBackup={backupStatus.canCreateBackup}
              canImportBackup={backupStatus.canImportBackup}
              canAutoSync={backupStatus.canAutoSync}
              showAutoMode={backupStatus.showAutoMode}
              handleUpload={backupStatus.handleUpload}
              handleImport={backupStatus.handleImport}
              runManualSync={backupStatus.runManualSync}
            />
          </CardContent>
        </Card>
      )}

      <ConfirmationDialog
        isOpen={isSignedIn && isLogoutConfirmationOpen}
        title={t.settings.cloud.logoutConfirmationTitle}
        message={t.settings.cloud.logoutConfirmationMessage}
        confirmText={t.settings.cloud.logout}
        cancelText={t.common.cancel}
        onConfirm={handleSignOut}
        onCancel={() => setIsLogoutConfirmationOpen(false)}
        isLoading={isLoading && activeAction === "signOut"}
      />
    </motion.div>
  )
}
