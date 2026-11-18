import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { openExternalAuthLink, openExternalAuthPage } from "@/lib/externalAuth"

const LINK = "https://auth.example.com/authorize?state=s1"

const assign = vi.fn()
const open = vi.fn()

beforeEach(() => {
  assign.mockReset()
  open.mockReset()
  vi.stubGlobal("location", { ...window.location, assign })
  vi.spyOn(window, "open").mockImplementation(open)
})

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe("openExternalAuthLink", () => {
  it("navigates the current tab", async () => {
    await openExternalAuthLink(LINK)

    expect(assign).toHaveBeenCalledExactlyOnceWith(LINK)
    expect(open).not.toHaveBeenCalled()
  })
})

describe("openExternalAuthPage", () => {
  it("opens a detached window", async () => {
    await openExternalAuthPage(LINK)

    expect(open).toHaveBeenCalledExactlyOnceWith(
      LINK,
      "_blank",
      "noopener,noreferrer",
    )
    expect(assign).not.toHaveBeenCalled()
  })

  it("ignores a failing window open", async () => {
    open.mockImplementation(() => {
      throw new Error("blocked")
    })

    await expect(openExternalAuthPage(LINK)).resolves.toBeUndefined()
  })
})
