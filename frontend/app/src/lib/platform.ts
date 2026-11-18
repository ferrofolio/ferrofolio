export function isPWAStandalone(): boolean {
  if (typeof window === "undefined") return false
  if ((navigator as { standalone?: boolean }).standalone) return true
  return window.matchMedia("(display-mode: standalone)").matches
}
