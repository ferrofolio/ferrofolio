// SPDX-License-Identifier: AGPL-3.0-only
import { TICKER_ICON_URL } from "@/env"

/**
 * URL of the icon for an ISIN or ticker, or null when no icon source is
 * configured. Icons are not loaded from upstream services by default.
 */
export function tickerIconUrl(
  token: string | null | undefined,
  baseUrl: string = TICKER_ICON_URL,
): string | null {
  const trimmed = token?.trim()
  const base = baseUrl?.trim().replace(/\/+$/, "")
  if (!trimmed || !base) return null
  return `${base}/${encodeURIComponent(trimmed)}.png`
}
