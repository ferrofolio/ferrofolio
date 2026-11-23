import { useState, useEffect } from "react"
import { Navigate, useSearchParams } from "react-router-dom"
import { useI18n, type Locale } from "@/i18n"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/Card"
import { Label } from "@/components/ui/Label"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/Tabs"
import { motion } from "framer-motion"
import {
  Sun,
  Moon,
  SunMoon,
  EyeOff,
  Settings2,
  SlidersHorizontal,
  Languages,
  Palette,
  Bot,
} from "lucide-react"
import { useAppContext, type SettingsKey } from "@/context/AppContext"
import { useTheme } from "@/context/ThemeContext"
import { LoadingSpinner } from "@/components/ui/LoadingSpinner"
import { GeneralTab } from "@/components/settings/GeneralTab"
import { McpTab } from "@/components/settings/McpTab"
import { AIModelsCard } from "@/components/ai/AIModelsCard"
import { AI_SETTINGS_TAB } from "@/components/ai/TaskAIProviders"
import { BottomNavCard } from "@/components/settings/BottomNavCard"
import { cn } from "@/lib/utils"
import { useDataDisplayMode } from "@/context/DataDisplayModeContext"
import { DataDisplayMode } from "@/types"

const APPLICATION_LOCALES: Locale[] = ["en-US", "es-ES", "it-IT", "de-DE"]
const LEGACY_MCP_TAB = "mcp"
const SETTINGS_TABS: string[] = ["general", "application", AI_SETTINGS_TAB]
const GENERAL_TAB_SETTINGS: SettingsKey[] = ["general", "data", "assets"]
export default function SettingsPage() {
  const { t, locale, changeLocale } = useI18n()
  const [searchParams] = useSearchParams()
  const { fetchSettings, isLoadingSettings, fetchExternalIntegrations } =
    useAppContext()
  const { theme, setThemeMode } = useTheme()
  const { mode: dataDisplayMode, setMode: setDataDisplayMode } =
    useDataDisplayMode()
  const [activeTab, setActiveTab] = useState(() => {
    const tab = searchParams.get("tab")
    if (tab === LEGACY_MCP_TAB) return AI_SETTINGS_TAB
    // An unknown tab selects no panel of the fixed tabs.
    return tab && SETTINGS_TABS.includes(tab) ? tab : "general"
  })

  const applicationLanguageOptions = APPLICATION_LOCALES.map(code => ({
    code,
    label: t.settings.applicationLanguageOptions[code],
  }))

  useEffect(() => {
    fetchSettings(GENERAL_TAB_SETTINGS)
    fetchExternalIntegrations()
  }, [])

  if (searchParams.get("tab") === "integrations") {
    const providerParams = new URLSearchParams(searchParams)
    providerParams.set("tab", "providers")
    return <Navigate to={`/entities?${providerParams.toString()}`} replace />
  }

  if (isLoadingSettings) {
    return (
      <div className="flex justify-center items-center h-64">
        <LoadingSpinner size="lg" />
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <div className="flex justify-between items-center">
        <h1 className="text-3xl font-bold">{t.settings.title}</h1>
      </div>

      <Tabs
        defaultValue="general"
        value={activeTab}
        onValueChange={setActiveTab}
        className="w-full"
      >
        <div className="flex justify-center w-full">
          <TabsList className="grid w-full max-w-[800px] h-auto min-h-[3rem] grid-cols-2 sm:grid-cols-3">
            <TabsTrigger
              value="general"
              className="text-xs sm:text-sm px-1 sm:px-2 py-2 whitespace-normal text-center leading-tight min-h-[2.5rem] flex items-center justify-center gap-1"
            >
              <Settings2 className="h-4 w-4 shrink-0" />
              {t.settings.general}
            </TabsTrigger>
            <TabsTrigger
              value="application"
              className="text-xs sm:text-sm px-1 sm:px-2 py-2 whitespace-normal text-center leading-tight min-h-[2.5rem] flex items-center justify-center gap-1"
            >
              <SlidersHorizontal className="h-4 w-4 shrink-0" />
              {t.settings.application}
            </TabsTrigger>
            <TabsTrigger
              value={AI_SETTINGS_TAB}
              className="text-xs sm:text-sm px-1 sm:px-2 py-2 whitespace-normal text-center leading-tight min-h-[2.5rem] flex items-center justify-center gap-1"
            >
              <Bot className="h-4 w-4 shrink-0" />
              {t.settings.aiTab.title}
            </TabsTrigger>
          </TabsList>
        </div>

        <TabsContent value="general" className="space-y-4 mt-4">
          <GeneralTab />
        </TabsContent>

        <TabsContent value="application" className="space-y-4 mt-4">
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            transition={{ duration: 0.3 }}
          >
            <div className="space-y-4">
              <p className="text-xs text-muted-foreground">
                {t.settings.applicationDisclaimerDescription}
              </p>
              <Card className="-mx-6 md:mx-0 rounded-none md:rounded-lg border-x-0 md:border-x">
                <CardHeader>
                  <div className="flex items-center gap-2">
                    <Languages className="h-5 w-5 text-primary" />
                    <CardTitle>{t.settings.applicationLanguageTitle}</CardTitle>
                  </div>
                  <CardDescription>
                    {t.settings.applicationLanguageDescription}
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="space-y-2">
                    <Label htmlFor="application-language">
                      {t.settings.applicationLanguageTitle}
                    </Label>
                    <select
                      id="application-language"
                      value={locale}
                      aria-label={t.settings.applicationLanguageTitle}
                      onChange={event => {
                        const nextLocale = event.target.value as Locale
                        if (nextLocale !== locale) {
                          changeLocale(nextLocale)
                        }
                      }}
                      className="flex h-10 w-full max-w-[200px] rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {applicationLanguageOptions.map(option => (
                        <option key={option.code} value={option.code}>
                          {option.label}
                        </option>
                      ))}
                    </select>
                  </div>
                </CardContent>
              </Card>

              <Card className="-mx-6 md:mx-0 rounded-none md:rounded-lg border-x-0 md:border-x">
                <CardHeader>
                  <div className="flex items-center gap-2">
                    <Palette className="h-5 w-5 text-primary" />
                    <CardTitle>{t.settings.themeTitle}</CardTitle>
                  </div>
                  <CardDescription>
                    {t.settings.themeDescription}
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="flex items-center gap-1 rounded-lg bg-muted p-1 w-fit">
                    <button
                      onClick={() => setThemeMode("light")}
                      className={cn(
                        "flex items-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
                        theme === "light"
                          ? "bg-background text-foreground shadow-sm"
                          : "text-muted-foreground hover:text-foreground",
                      )}
                    >
                      <Sun className="h-4 w-4" />
                      <span
                        className={cn(
                          theme !== "light" && "hidden min-[400px]:inline",
                        )}
                      >
                        {t.common.light}
                      </span>
                    </button>
                    <button
                      onClick={() => setThemeMode("dark")}
                      className={cn(
                        "flex items-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
                        theme === "dark"
                          ? "bg-background text-foreground shadow-sm"
                          : "text-muted-foreground hover:text-foreground",
                      )}
                    >
                      <Moon className="h-4 w-4" />
                      <span
                        className={cn(
                          theme !== "dark" && "hidden min-[400px]:inline",
                        )}
                      >
                        {t.common.dark}
                      </span>
                    </button>
                    <button
                      onClick={() => setThemeMode("system")}
                      className={cn(
                        "flex items-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
                        theme === "system"
                          ? "bg-background text-foreground shadow-sm"
                          : "text-muted-foreground hover:text-foreground",
                      )}
                    >
                      <SunMoon className="h-4 w-4" />
                      <span
                        className={cn(
                          theme !== "system" && "hidden min-[400px]:inline",
                        )}
                      >
                        {t.common.system}
                      </span>
                    </button>
                  </div>
                </CardContent>
              </Card>

              <Card className="-mx-6 md:mx-0 rounded-none md:rounded-lg border-x-0 md:border-x">
                <CardHeader>
                  <div className="flex items-center gap-2">
                    <EyeOff className="h-5 w-5 text-primary" />
                    <CardTitle>{t.settings.dataDisplayModeTitle}</CardTitle>
                  </div>
                  <CardDescription>
                    {t.settings.dataDisplayModeDescription}
                  </CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="flex items-center gap-1 rounded-lg bg-muted p-1 w-fit">
                    <button
                      onClick={() => setDataDisplayMode(DataDisplayMode.NONE)}
                      className={cn(
                        "flex items-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
                        dataDisplayMode === DataDisplayMode.NONE
                          ? "bg-background text-foreground shadow-sm"
                          : "text-muted-foreground hover:text-foreground",
                      )}
                    >
                      {t.settings.dataDisplayModeNone}
                    </button>
                    <button
                      onClick={() =>
                        setDataDisplayMode(DataDisplayMode.PRIVATE)
                      }
                      className={cn(
                        "flex items-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
                        dataDisplayMode === DataDisplayMode.PRIVATE
                          ? "bg-background text-foreground shadow-sm"
                          : "text-muted-foreground hover:text-foreground",
                      )}
                    >
                      <EyeOff className="h-4 w-4" />
                      {t.settings.dataDisplayModePrivate}
                    </button>
                  </div>
                </CardContent>
              </Card>

              <BottomNavCard />
            </div>
          </motion.div>
        </TabsContent>

        <TabsContent value={AI_SETTINGS_TAB} className="space-y-4 mt-4">
          <AIModelsCard />
          <McpTab />
        </TabsContent>
      </Tabs>
    </div>
  )
}
