import { useState } from "react"
import { Button } from "@/components/ui/Button"
import {
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  CardFooter,
} from "@/components/ui/Card"
import { useI18n } from "@/i18n"
import { GitHubRelease } from "@/types/release"
import { formatReleaseDate, cleanReleaseNotes } from "@/utils/releaseUtils"
import { ExternalLink, Calendar, Package } from "lucide-react"
import { motion, AnimatePresence } from "framer-motion"
import { ReleaseNotes } from "@/components/ReleaseNotes"

interface ReleaseUpdateModalProps {
  isOpen: boolean
  onClose: () => void
  currentVersion: string
  latestVersion: string
  release: GitHubRelease
  onSkipVersion?: (version: string) => void
}

export function ReleaseUpdateModal({
  isOpen,
  onClose,
  currentVersion,
  latestVersion,
  release,
  onSkipVersion,
}: ReleaseUpdateModalProps) {
  const { t, locale } = useI18n()
  const [showFullNotes, setShowFullNotes] = useState(false)

  if (!isOpen) return null

  const releaseDate = formatReleaseDate(release.published_at, locale)
  const notes = cleanReleaseNotes(release.body)

  const handleViewRelease = () => {
    window.open(release.html_url, "_blank")
  }

  const handleSkipVersion = () => {
    if (onSkipVersion) {
      onSkipVersion(latestVersion)
    }
    onClose()
  }

  // Truncate release notes for preview
  const truncateText = (text: string, maxLength: number = 500): string => {
    if (text.length <= maxLength) return text
    return text.substring(0, maxLength) + "..."
  }

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center p-2 sm:p-4 z-50">
      <AnimatePresence>
        <motion.div
          initial={{ opacity: 0, scale: 0.9 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{ opacity: 0, scale: 0.9 }}
          transition={{ duration: 0.2 }}
          className="w-full"
        >
          <Card className="w-full max-w-2xl mx-auto max-h-[95vh] sm:max-h-[90vh] flex flex-col">
            <CardHeader className="flex-shrink-0 px-4 pt-4 pb-0 sm:pt-6 sm:px-6">
              <CardTitle className="flex items-center gap-2 text-green-600 dark:text-green-400 text-lg sm:text-xl">
                <Package className="h-5 w-5 flex-shrink-0" />
                {t.release.newVersionAvailable}
              </CardTitle>
              <div className="flex flex-col gap-2 text-sm text-muted-foreground">
                <div className="flex flex-col sm:flex-row sm:items-center gap-2 sm:gap-4">
                  <span className="text-xs sm:text-sm">
                    {t.release.currentVersion}:{" "}
                    <code className="bg-muted px-1 rounded text-xs">
                      {currentVersion}
                    </code>
                  </span>
                  <span className="text-xs sm:text-sm">
                    {t.release.latestVersion}:{" "}
                    <code className="bg-green-100 dark:bg-green-900 px-1 rounded text-xs text-green-700 dark:text-green-300">
                      {latestVersion}
                    </code>
                  </span>
                </div>
                <div className="flex items-center gap-1">
                  <Calendar className="h-4 w-4 flex-shrink-0" />
                  <span className="text-xs sm:text-sm">
                    {t.release.publishedOn} {releaseDate}
                  </span>
                </div>
              </div>
            </CardHeader>

            <CardContent className="flex-1 overflow-y-auto min-h-0 space-y-4 p-4 sm:p-6">
              <div className="space-y-3">
                <h4 className="font-medium text-sm">
                  {t.release.downloadAssets}
                </h4>
                <div className="p-3 bg-muted rounded-lg">
                  <p className="text-sm text-muted-foreground mb-3">
                    {t.release.releasePageLink}
                  </p>
                  <Button
                    onClick={handleViewRelease}
                    size="sm"
                    className="w-full sm:w-auto"
                  >
                    <ExternalLink className="h-4 w-4 mr-1" />
                    {t.common.viewRelease}
                  </Button>
                </div>
              </div>

              {/* Release Notes */}
              <div className="space-y-3">
                <h4 className="font-medium text-sm">{t.release.whatsNew}</h4>
                <div className="bg-muted p-3 sm:p-4 rounded-lg">
                  <ReleaseNotes
                    content={showFullNotes ? notes : truncateText(notes)}
                  />
                  {notes.length > 500 && (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setShowFullNotes(!showFullNotes)}
                      className="mt-3 text-xs"
                    >
                      {showFullNotes ? "Show Less" : "Show More"}
                    </Button>
                  )}
                </div>
              </div>
            </CardContent>

            <CardFooter className="flex-shrink-0 flex flex-col sm:flex-row sm:justify-between gap-3 p-4 sm:p-6 pt-6">
              <div className="flex flex-col sm:flex-row gap-2 w-full sm:w-auto">
                <Button
                  variant="outline"
                  onClick={onClose}
                  className="w-full sm:w-auto"
                >
                  {t.common.remindLater}
                </Button>
                {onSkipVersion && (
                  <Button
                    variant="ghost"
                    onClick={handleSkipVersion}
                    size="sm"
                    className="w-full sm:w-auto"
                  >
                    {t.release.skipThisVersion}
                  </Button>
                )}
              </div>
              <div className="flex flex-col sm:flex-row gap-2 w-full sm:w-auto">
                <Button
                  variant="outline"
                  onClick={handleViewRelease}
                  className="w-full sm:w-auto"
                >
                  <ExternalLink className="h-4 w-4 mr-1" />
                  {t.common.viewRelease}
                </Button>
                <Button onClick={onClose} className="w-full sm:w-auto">
                  {t.common.close}
                </Button>
              </div>
            </CardFooter>
          </Card>
        </motion.div>
      </AnimatePresence>
    </div>
  )
}
