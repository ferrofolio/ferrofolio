import { afterEach, describe, expect, it, vi } from "vitest"
import { cleanup, render, screen } from "@testing-library/react"
import { AboutWindow } from "@/about/AboutWindow"
import en from "@/i18n/locales/en.json"
import type { AboutAppInfo } from "@/types"

vi.mock("@/i18n", () => ({ useI18n: () => ({ t: en, locale: "en-US" }) }))

describe("AboutWindow", () => {
  afterEach(() => {
    cleanup()
    delete (window as { ipcAPI?: unknown }).ipcAPI
  })

  it("shows the localized product name, not the packaged app name", async () => {
    // A packaged build reports the productName from the upstream build config.
    const info: AboutAppInfo = { appName: "Finanze", version: "1.2.3" }
    ;(window as { ipcAPI?: unknown }).ipcAPI = {
      getAboutInfo: vi.fn().mockResolvedValue(info),
    }

    render(<AboutWindow />)

    expect(await screen.findByText("1.2.3")).toBeInTheDocument()
    expect(screen.getByText("About Ferrofolio")).toBeInTheDocument()
    expect(screen.getByAltText("Ferrofolio logo")).toBeInTheDocument()
    expect(document.title).toBe("About Ferrofolio")
    expect(document.body.textContent).not.toMatch(/\bFinanze\b/)
  })
})
