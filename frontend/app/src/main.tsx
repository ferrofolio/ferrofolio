import React from "react"
import { createRoot } from "react-dom/client"
import { HashRouter } from "react-router-dom"
import App from "./App"
import "./index.css"
import { AppProvider } from "@/context/AppContext"
import { I18nProvider } from "@/i18n"
import { ThemeProvider } from "@/context/ThemeContext"
import { AuthProvider } from "@/context/AuthContext"
import { CloudProvider } from "@/context/CloudContext"
import { BackupAlertProvider } from "@/context/BackupAlertContext"
import { ModalRegistryProvider } from "@/context/ModalRegistryContext"
import { DataDisplayModeProvider } from "@/context/DataDisplayModeContext"
import { initE2eMockExternalLogin } from "@/lib/dev/initE2eMockExternalLogin"
import { initE2eMockChallengeWindow } from "@/lib/dev/initE2eMockChallengeWindow"
import { ErrorBoundary } from "@/components/ErrorBoundary"
import { initTelemetry } from "@/lib/telemetry"
import { registerServiceWorker } from "@/lib/serviceWorker"
import { ServiceWorkerUpdatePrompt } from "@/components/ServiceWorkerUpdatePrompt"

async function bootstrap(): Promise<void> {
  initE2eMockExternalLogin()
  initE2eMockChallengeWindow()

  await initTelemetry().catch(console.error)

  createRoot(document.getElementById("root")!).render(
    <HashRouter>
      <ThemeProvider>
        <DataDisplayModeProvider>
          <I18nProvider>
            <ErrorBoundary scope="app">
              <ModalRegistryProvider>
                <AuthProvider>
                  <AppProvider>
                    <CloudProvider>
                      <BackupAlertProvider>
                        <App />
                      </BackupAlertProvider>
                    </CloudProvider>
                  </AppProvider>
                </AuthProvider>
              </ModalRegistryProvider>
              <ServiceWorkerUpdatePrompt />
            </ErrorBoundary>
          </I18nProvider>
        </DataDisplayModeProvider>
      </ThemeProvider>
    </HashRouter>,
  )

  if (import.meta.env.PROD) {
    registerServiceWorker().catch(console.error)
  }
}

bootstrap()
