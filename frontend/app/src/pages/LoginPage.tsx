import type React from "react"

import { useState, useEffect, useMemo } from "react"
import { Button } from "@/components/ui/Button"
import { Label } from "@/components/ui/Label"
import { LoadingSpinner } from "@/components/ui/LoadingSpinner"
import { Switch } from "@/components/ui/Switch"
import { motion } from "framer-motion"
import { useAuth } from "@/context/AuthContext"
import {
  Wrench,
  ArrowLeft,
  Copy,
  Check,
  Info,
  Cloud,
  ShieldOff,
} from "lucide-react"
import { useI18n } from "@/i18n"
import { cn } from "@/lib/utils"
import { useAppContext } from "@/context/AppContext"
import { AuthResultCode } from "@/types"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/Popover"
import { LoginQuickSettings } from "@/components/ui/ThemeSelector"
import { useTheme } from "@/context/ThemeContext"
import { generateFloatingLogos } from "@/lib/floatingLogos"
import { copyToClipboard } from "@/lib/clipboard"
import { CloudRestore } from "@/components/auth/CloudRestore"
import {
  authInputClass,
  authPasswordValueClass,
  isLightTheme,
} from "@/components/auth/authStyles"
import { ErrorReportingToggle } from "@/components/telemetry/ErrorReportingToggle"
import {
  TelemetryConsentDialog,
  useErrorReportingConsent,
} from "@/components/telemetry/TelemetryConsentDialog"

const USERNAME_PATTERN = /^[a-zA-Z0-9_\-.$€#]{2,}$/
const PASSWORD_PATTERN = /^[\x21-\x7e]{8,}$/
const REMEMBER_SESSION_KEY = "finanze_remember_session"

export default function LoginPage() {
  const [username, setUsername] = useState("")
  const [password, setPassword] = useState("")
  const [oldPassword, setOldPassword] = useState("")
  const [repeatPassword, setRepeatPassword] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [errorCode, setErrorCode] = useState<AuthResultCode | null>(null)
  const [errorDetails, setErrorDetails] = useState<string | null>(null)
  const [isSignupMode, setIsSignupMode] = useState(false)
  const [detailsCopied, setDetailsCopied] = useState(false)
  const [cloudRestoreMode, setCloudRestoreMode] = useState(false)
  const [showConsentDialog, setShowConsentDialog] = useState(false)
  const [rememberSession, setRememberSession] = useState(
    () => localStorage.getItem(REMEMBER_SESSION_KEY) === "true",
  )

  const {
    login,
    signup,
    changePassword,
    isLoading,
    lastLoggedUser,
    pendingRegister,
    isChangingPassword,
    pendingPasswordChangeUser,
    cancelPasswordChange,
    apiSessionEnabled,
  } = useAuth()
  const { showToast } = useAppContext()
  const { t } = useI18n()
  const { theme } = useTheme()
  const { enabled: errorReportingEnabled, refresh: refreshConsent } =
    useErrorReportingConsent()

  const isLight = isLightTheme(theme)

  const floatingLogos = useMemo(() => {
    const isWide = window.innerWidth >= 900
    const count = isWide ? 280 : 80
    return generateFloatingLogos(count, count * 12, isWide ? 0.55 : 1.2)
  }, [])

  const isLoginMode = !isSignupMode && !isChangingPassword

  const showRememberOption = apiSessionEnabled && !isChangingPassword
  const remember = showRememberOption && rememberSession

  const handleRememberSessionChange = (checked: boolean) => {
    setRememberSession(checked)
    localStorage.setItem(REMEMBER_SESSION_KEY, String(checked))
  }

  useEffect(() => {
    if (isChangingPassword) {
      const userForDisplay = pendingPasswordChangeUser || lastLoggedUser
      if (userForDisplay) {
        setUsername(userForDisplay)
      }
      setIsSignupMode(false)
    } else if (pendingRegister && lastLoggedUser) {
      setUsername(lastLoggedUser)
      setIsSignupMode(true)
    } else if (lastLoggedUser) {
      setUsername(lastLoggedUser)
      setIsSignupMode(false)
    } else {
      setIsSignupMode(true)
    }
  }, [
    lastLoggedUser,
    isChangingPassword,
    pendingPasswordChangeUser,
    pendingRegister,
  ])

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setError(null)
    setErrorCode(null)
    setErrorDetails(null)

    if ((isSignupMode || isChangingPassword) && password !== repeatPassword) {
      setError(t.login.passwordsDontMatch)
      return
    }

    if (isSignupMode && !USERNAME_PATTERN.test(username)) {
      setError(t.login.invalidUsername)
      return
    }

    if (
      (isSignupMode || isChangingPassword) &&
      !PASSWORD_PATTERN.test(password)
    ) {
      setError(t.login.invalidPasswordFormat)
      return
    }

    try {
      if (isChangingPassword) {
        const result = await changePassword(oldPassword, password)
        if (result) {
          showToast(t.login.changePasswordSuccess, "success")
          setOldPassword("")
          setPassword("")
          setRepeatPassword("")
          setError(null)
        }
      } else if (isSignupMode) {
        const signupResult = await signup(username, password, remember)
        if (!signupResult) {
          setError(t.login.invalidCredentials)
        }
      } else {
        const { code, message } = await login(username, password, remember)
        switch (code) {
          case AuthResultCode.SUCCESS:
            break
          case AuthResultCode.INVALID_CREDENTIALS:
            setError(t.login.invalidCredentials)
            setErrorCode(code)
            break
          case AuthResultCode.USER_NOT_FOUND:
            setError(t.login.userNotFound)
            setErrorCode(code)
            break
          case AuthResultCode.UNEXPECTED_ERROR:
          default:
            setError(t.login.unexpectedErrorContact)
            setErrorCode(AuthResultCode.UNEXPECTED_ERROR)
            if (message) {
              setErrorDetails(message)
            }
            break
        }
      }
    } catch (error: any) {
      if (isChangingPassword) {
        if (
          error.status === 401 ||
          error.message?.includes("401") ||
          error.message?.toLowerCase().includes("unauthorized") ||
          error.message?.toLowerCase().includes("invalid")
        ) {
          setError(t.login.invalidCredentials)
          showToast(t.login.invalidCredentials, "error", { reportable: false })
        } else {
          const errorMessage = error.message || t.login.changePasswordError
          setError(errorMessage)
          showToast(errorMessage, "error")
        }
      } else {
        setError(t.login.unexpectedErrorContact)
        setErrorCode(AuthResultCode.UNEXPECTED_ERROR)
        if (error?.message) {
          setErrorDetails(error.message)
        }
      }
    }
  }

  const handleCancelPasswordChange = () => {
    cancelPasswordChange()
    setError(null)
    setErrorCode(null)
    setErrorDetails(null)
    setOldPassword("")
    setPassword("")
    setRepeatPassword("")
    setIsSignupMode(false)
  }

  const getTitle = () => {
    if (isChangingPassword) {
      return t.login.changePasswordTitle
    } else if (isSignupMode) {
      return t.login.signupTitle
    } else if (lastLoggedUser) {
      return t.login.welcomeBack.replace("{username}", lastLoggedUser)
    } else {
      return t.login.title
    }
  }

  const inputClass = authInputClass(isLight)
  const passwordValueClass = authPasswordValueClass

  const renderConsentToggle = (textClass: string) => {
    if (!isSignupMode || isChangingPassword) return null

    return <ErrorReportingToggle textClass={textClass} className="shrink-0" />
  }

  const renderConsentQuickAction = () => {
    if (!isLoginMode || errorReportingEnabled !== false) return null

    return (
      <Button
        variant="ghost"
        size="icon"
        className="rounded-full h-10 w-10 hover:bg-gray-200 dark:hover:bg-gray-800"
        onClick={() => setShowConsentDialog(true)}
        aria-label={t.telemetryConsent.reportAction}
        title={t.telemetryConsent.reportAction}
        data-testid="login-enable-error-reporting"
      >
        <ShieldOff size={18} />
      </Button>
    )
  }

  const consentDialog = (
    <TelemetryConsentDialog
      isOpen={showConsentDialog}
      onClose={() => setShowConsentDialog(false)}
      onEnabled={refreshConsent}
      reason="manual"
    />
  )

  return (
    <div
      className={`relative min-h-screen flex items-center justify-center p-4 select-none ${
        isLight ? "bg-white" : "bg-black"
      }`}
    >
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ duration: 1.1, ease: "easeOut" }}
        className="absolute inset-0 pointer-events-none overflow-hidden"
        style={{
          maskImage:
            "linear-gradient(to bottom, rgba(0,0,0,1) 0%, rgba(0,0,0,0.45) 55%, transparent 100%)",
          WebkitMaskImage:
            "linear-gradient(to bottom, rgba(0,0,0,1) 0%, rgba(0,0,0,0.45) 55%, transparent 100%)",
        }}
      >
        {floatingLogos.map((pos, i) => (
          <img
            key={i}
            src="ferrofolio-fg.svg"
            alt=""
            className="absolute select-none pointer-events-none"
            style={{
              left: `${pos.x}%`,
              top: `${pos.y}%`,
              width: pos.size,
              height: pos.size,
              opacity: pos.opacity,
              transform: `rotate(${pos.rotation}deg)`,
            }}
            draggable={false}
          />
        ))}
      </motion.div>
      <div className="absolute bottom-6 left-6 z-10 flex items-center gap-2">
        <LoginQuickSettings />
        {renderConsentQuickAction()}
      </div>
      <motion.div
        initial={{ opacity: 0, y: 20 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.5 }}
        className="relative w-full max-w-sm flex flex-col items-center px-10 z-10"
      >
        {isChangingPassword && (
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className={`absolute left-0 top-0 ${isLight ? "text-black" : "text-white"}`}
            onClick={handleCancelPasswordChange}
            disabled={isLoading}
            aria-label={t.common.cancel}
          >
            <ArrowLeft className="h-6 w-6" strokeWidth={2.75} />
          </Button>
        )}
        <img
          src="ferrofolio-fg.svg"
          alt={t.common.logoAlt}
          className="select-none pointer-events-none mb-5"
          style={{ width: 56, height: 56 }}
          draggable={false}
        />
        <h1
          className={`text-xl font-medium text-center mb-10 tracking-tight ${isLight ? "text-black" : "text-white"}`}
        >
          {cloudRestoreMode ? t.login.cloudRestore.title : getTitle()}
        </h1>
        {cloudRestoreMode ? (
          <div className="w-full">
            <p
              className={`text-center text-sm mb-6 ${isLight ? "text-black/50" : "text-white/50"}`}
            >
              {t.login.cloudRestore.description}
            </p>
            <CloudRestore
              onBack={() => setCloudRestoreMode(false)}
              onRestoreComplete={() => {}}
              pendingUsername={
                pendingRegister ? lastLoggedUser || undefined : undefined
              }
            />
          </div>
        ) : (
          <form
            onSubmit={handleSubmit}
            className="w-full space-y-6"
            noValidate={isChangingPassword}
          >
            {!isChangingPassword && (isSignupMode || !lastLoggedUser) && (
              <div className="relative">
                <input
                  id="username"
                  type="text"
                  value={username}
                  onChange={e => setUsername(e.target.value)}
                  placeholder={t.login.namePlaceholder}
                  required={!isChangingPassword}
                  autoFocus={isSignupMode || !lastLoggedUser}
                  autoCapitalize="off"
                  disabled={pendingRegister}
                  className={inputClass}
                />
                {isSignupMode && (
                  <Popover>
                    <PopoverTrigger asChild>
                      <button
                        type="button"
                        className="absolute right-0 top-1/2 -translate-y-1/2 p-2 text-muted-foreground hover:text-foreground transition-colors"
                        aria-label={t.login.usernameInfoTooltip}
                      >
                        <Info className="h-4 w-4" />
                      </button>
                    </PopoverTrigger>
                    <PopoverContent className="w-56 text-sm" sideOffset={8}>
                      <p className="text-muted-foreground">
                        {t.login.usernameInfoTooltip}
                      </p>
                    </PopoverContent>
                  </Popover>
                )}
              </div>
            )}

            {isChangingPassword && (
              <input
                id="oldPassword"
                type="password"
                value={oldPassword}
                onChange={e => setOldPassword(e.target.value)}
                placeholder={t.login.oldPasswordPlaceholder}
                required
                autoFocus
                className={cn(inputClass, passwordValueClass)}
              />
            )}

            <input
              id="password"
              type="password"
              value={password}
              onChange={e => setPassword(e.target.value)}
              placeholder={
                isChangingPassword
                  ? t.login.newPasswordPlaceholder
                  : t.login.passwordPlaceholder
              }
              required
              autoFocus={
                !isSignupMode && !isChangingPassword && !!lastLoggedUser
              }
              className={cn(inputClass, passwordValueClass)}
            />

            {(isSignupMode || isChangingPassword) && password.length > 0 && (
              <motion.div
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: "auto" }}
                transition={{ duration: 0.25 }}
              >
                <input
                  id="repeatPassword"
                  type="password"
                  value={repeatPassword}
                  onChange={e => setRepeatPassword(e.target.value)}
                  placeholder={t.login.repeatPasswordPlaceholder}
                  required
                  className={cn(inputClass, passwordValueClass)}
                />
              </motion.div>
            )}

            {showRememberOption && (
              <div className="flex items-center justify-between py-2">
                <Label
                  htmlFor="rememberSession"
                  className="text-sm cursor-pointer"
                >
                  {t.login.rememberSession}
                </Label>
                <Switch
                  id="rememberSession"
                  checked={rememberSession}
                  onCheckedChange={handleRememberSessionChange}
                />
              </div>
            )}

            {error && (
              <motion.div
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                className="text-sm text-red-500 dark:text-red-400 text-center flex items-center justify-center"
              >
                {error}
                {errorCode === AuthResultCode.UNEXPECTED_ERROR &&
                  errorDetails && (
                    <Popover>
                      <PopoverTrigger asChild>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="ml-2 h-7 w-7 text-red-500"
                          aria-label={t.login.viewErrorDetails}
                        >
                          <Wrench className="h-4 w-4" />
                        </Button>
                      </PopoverTrigger>
                      <PopoverContent
                        className="w-64 max-h-[50vh] overflow-y-auto text-left text-sm"
                        sideOffset={8}
                      >
                        <div className="flex items-center justify-between">
                          <p className="font-medium text-foreground">
                            {t.login.errorDetailsTitle}
                          </p>
                          <button
                            type="button"
                            className="p-1 text-muted-foreground hover:text-foreground transition-colors"
                            onClick={async () => {
                              if (await copyToClipboard(errorDetails || "")) {
                                setDetailsCopied(true)
                                setTimeout(() => setDetailsCopied(false), 2000)
                              }
                            }}
                          >
                            {detailsCopied ? (
                              <Check className="h-3.5 w-3.5 text-green-500" />
                            ) : (
                              <Copy className="h-3.5 w-3.5" />
                            )}
                          </button>
                        </div>
                        <p className="mt-2 text-muted-foreground break-words">
                          {errorDetails}
                        </p>
                      </PopoverContent>
                    </Popover>
                  )}
              </motion.div>
            )}

            <div className="flex items-center gap-2">
              <Button
                type="submit"
                variant="ghost"
                className={cn(
                  "flex-1 min-w-0 text-lg py-6 font-bold bg-transparent shadow-none hover:bg-transparent dark:bg-transparent dark:hover:bg-transparent",
                  isLight
                    ? "text-black hover:text-black"
                    : "text-white hover:text-white dark:text-white dark:hover:text-white",
                )}
                disabled={isLoading}
              >
                {isLoading ? (
                  <>
                    <LoadingSpinner size="sm" className="mr-2" />
                    {t.common.loading}
                  </>
                ) : isChangingPassword ? (
                  t.login.changePassword
                ) : isSignupMode ? (
                  t.login.signup
                ) : (
                  t.common.unlock
                )}
              </Button>
              {renderConsentToggle("text-foreground")}
            </div>

            {(isSignupMode || isChangingPassword) && (
              <p className="text-xs text-muted-foreground text-center mt-4">
                {t.login.syncPasswordHint}
              </p>
            )}

            {isSignupMode && !isChangingPassword && (
              <button
                type="button"
                onClick={() => setCloudRestoreMode(true)}
                className="w-full flex items-center justify-center gap-2 text-sm text-muted-foreground hover:text-foreground transition-colors mt-2"
              >
                <Cloud className="h-4 w-4" />
                {t.login.cloudRestore.trigger}
              </button>
            )}
          </form>
        )}
      </motion.div>
      {consentDialog}
    </div>
  )
}
