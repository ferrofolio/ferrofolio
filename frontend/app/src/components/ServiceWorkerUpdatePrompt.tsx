import { useEffect, useState } from "react"
import { Toast } from "@/components/ui/Toast"
import { useI18n } from "@/i18n"
import {
  applyServiceWorkerUpdate,
  subscribeToServiceWorkerUpdate,
} from "@/lib/serviceWorker"

// Asks the user to reload when a new version of the app has been installed in the background.
export function ServiceWorkerUpdatePrompt() {
  const { t } = useI18n()
  const [updateAvailable, setUpdateAvailable] = useState(false)
  const [dismissed, setDismissed] = useState(false)

  useEffect(() => subscribeToServiceWorkerUpdate(setUpdateAvailable), [])

  if (!updateAvailable || dismissed) return null

  return (
    <Toast
      variant="info"
      role="status"
      className="bottom-auto top-[max(1rem,env(safe-area-inset-top))]"
      onClose={() => setDismissed(true)}
      closeLabel={t.common.close}
      action={
        <button
          type="button"
          onClick={applyServiceWorkerUpdate}
          className="shrink-0 rounded px-2 py-0.5 text-sm font-medium text-primary transition-colors hover:underline focus:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          {t.common.reload}
        </button>
      }
    >
      {t.common.newVersionReady}
    </Toast>
  )
}
