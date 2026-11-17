import { PlatformInfo, PlatformType } from "@/types"

export function getPlatformInfo(): PlatformInfo {
  return window.platform ?? { type: PlatformType.WEB }
}

export function getPlatformType(): PlatformType {
  return getPlatformInfo().type
}

export function isElectron(): boolean {
  return typeof window !== "undefined" && typeof window.ipcAPI !== "undefined"
}

export function isWeb(): boolean {
  return !isElectron()
}

export function isPWAStandalone(): boolean {
  if (typeof window === "undefined") return false
  if ((navigator as { standalone?: boolean }).standalone) return true
  return window.matchMedia("(display-mode: standalone)").matches
}
