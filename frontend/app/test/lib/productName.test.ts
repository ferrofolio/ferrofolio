// SPDX-License-Identifier: AGPL-3.0-only
import { readFileSync } from "node:fs"
import { resolve } from "node:path"
import { describe, expect, it } from "vitest"

import de from "@/i18n/locales/de.json"
import en from "@/i18n/locales/en.json"
import es from "@/i18n/locales/es.json"
import it_ from "@/i18n/locales/it.json"

function namedStrings(value: unknown, key = ""): [string, string][] {
  if (typeof value === "string") return [[key, value]]
  if (value && typeof value === "object") {
    return Object.entries(value).flatMap(([k, v]) => namedStrings(v, k))
  }
  return []
}

describe("product name", () => {
  it.each([
    ["en", en],
    ["de", de],
    ["es", es],
    ["it", it_],
  ])("is Ferrofolio in the %s locale", (_, locale) => {
    const offending = namedStrings(locale).filter(([, text]) =>
      /\bFinanze\b/.test(text),
    )
    expect(offending).toEqual([])
    expect(locale.login.title).toContain("Ferrofolio")
    expect(locale.common.appName).toBe("Ferrofolio")
    expect(locale.common.logoAlt).toContain("Ferrofolio")
  })

  it("is Ferrofolio in the page title and the web manifest", () => {
    const root = resolve(__dirname, "../..")
    const manifest = JSON.parse(
      readFileSync(resolve(root, "public/site.webmanifest"), "utf8"),
    )
    expect(manifest.name).toBe("Ferrofolio")
    expect(manifest.short_name).toBe("Ferrofolio")
    expect(readFileSync(resolve(root, "index.html"), "utf8")).toContain(
      "<title>Ferrofolio</title>",
    )
  })
})
