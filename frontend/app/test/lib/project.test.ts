// SPDX-License-Identifier: AGPL-3.0-only
import { describe, expect, it } from "vitest"

import de from "@/i18n/locales/de.json"
import en from "@/i18n/locales/en.json"
import es from "@/i18n/locales/es.json"
import it_ from "@/i18n/locales/it.json"
import { LICENSE_URL, SOURCE_CODE_LABEL, SOURCE_CODE_URL } from "@/lib/project"

describe("project links", () => {
  it("point to the Ferrofolio repository", () => {
    expect(SOURCE_CODE_URL).toBe("https://github.com/ferrofolio/ferrofolio")
    expect(SOURCE_CODE_URL).toContain(SOURCE_CODE_LABEL)
    expect(LICENSE_URL.startsWith(`${SOURCE_CODE_URL}/`)).toBe(true)
    expect(LICENSE_URL.endsWith("/LICENSE")).toBe(true)
  })

  it.each([
    ["en", en],
    ["de", de],
    ["es", es],
    ["it", it_],
  ])("are labelled in the %s locale", (_, locale) => {
    const legal = locale.settings.legal as Record<string, string>
    expect(legal.sourceCode).toBeTruthy()
    expect(legal.license).toContain("AGPL-3.0")
  })
})
