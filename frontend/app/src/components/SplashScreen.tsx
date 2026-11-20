import { useI18n } from "@/i18n"
import { LoginQuickSettings } from "@/components/ui/ThemeSelector"
import { useTheme } from "@/context/ThemeContext"

interface SplashScreenProps {
  hasConnectionFailed?: boolean
}

export default function SplashScreen({
  hasConnectionFailed = false,
}: SplashScreenProps) {
  const { t } = useI18n()
  const { theme } = useTheme()

  const isLight =
    theme === "light" ||
    (theme === "system" &&
      typeof window !== "undefined" &&
      !window.matchMedia("(prefers-color-scheme: dark)").matches)

  const gradientClass = isLight
    ? "from-gray-400 via-gray-700 to-gray-300"
    : "from-gray-800 via-white to-gray-700"

  return (
    <div
      className={`select-none min-h-screen flex flex-col items-center justify-center ${
        isLight ? "bg-white" : "bg-black"
      }`}
    >
      <img
        src="ferrofolio-fg.svg"
        alt={t.common.logoAlt}
        className="w-32 h-32 animate-breathing drop-shadow select-none pointer-events-none"
        draggable={false}
      />
      <div className="absolute bottom-20 flex flex-col items-center gap-3">
        {hasConnectionFailed ? (
          <>
            <p
              className={`text-lg font-bold bg-gradient-to-r ${gradientClass} bg-[length:200%_auto] animate-text-shine bg-clip-text text-transparent`}
            >
              {t.common.connectingToServer}
            </p>
            <p className="text-sm text-gray-400 max-w-xs text-center">
              {t.common.serverCannotReach}
            </p>
          </>
        ) : (
          <p
            className={`text-lg font-bold bg-gradient-to-r ${gradientClass} bg-[length:200%_auto] animate-text-shine bg-clip-text text-transparent`}
          >
            {t.common.connectingToServer}
          </p>
        )}
      </div>
      <div
        className="absolute left-6"
        style={{
          bottom:
            "calc(24px + max(calc(var(--safe-area-inset-bottom, 0px) - 24px), 0px))",
        }}
      >
        <LoginQuickSettings />
      </div>
    </div>
  )
}
