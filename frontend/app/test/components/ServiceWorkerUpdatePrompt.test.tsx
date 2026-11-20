import { afterEach, describe, expect, it, vi } from "vitest"
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react"
import { ServiceWorkerUpdatePrompt } from "@/components/ServiceWorkerUpdatePrompt"

let notify: (available: boolean) => void = () => undefined
const applyUpdate = vi.fn()

vi.mock("@/lib/serviceWorker", () => ({
  subscribeToServiceWorkerUpdate: (listener: (available: boolean) => void) => {
    notify = listener
    listener(false)
    return () => undefined
  },
  applyServiceWorkerUpdate: () => applyUpdate(),
}))

vi.mock("@/i18n", () => ({
  useI18n: () => ({
    t: {
      common: {
        reload: "Reload",
        newVersionReady: "A new version of Ferrofolio is ready.",
      },
    },
    locale: "en-US",
  }),
}))

describe("ServiceWorkerUpdatePrompt", () => {
  afterEach(() => {
    cleanup()
    applyUpdate.mockReset()
  })

  it("stays hidden while no update is waiting", () => {
    render(<ServiceWorkerUpdatePrompt />)
    expect(
      screen.queryByText("A new version of Ferrofolio is ready."),
    ).toBeNull()
  })

  it("asks to reload once an update is waiting", () => {
    render(<ServiceWorkerUpdatePrompt />)
    act(() => notify(true))
    expect(
      screen.getByText("A new version of Ferrofolio is ready."),
    ).toBeInTheDocument()

    fireEvent.click(screen.getByText("Reload"))
    expect(applyUpdate).toHaveBeenCalledOnce()
  })

  it("can be dismissed without reloading", () => {
    render(<ServiceWorkerUpdatePrompt />)
    act(() => notify(true))
    const close = screen
      .getAllByRole("button")
      .find(button => button.textContent !== "Reload")!
    fireEvent.click(close)
    expect(
      screen.queryByText("A new version of Ferrofolio is ready."),
    ).toBeNull()
    expect(applyUpdate).not.toHaveBeenCalled()
  })
})
