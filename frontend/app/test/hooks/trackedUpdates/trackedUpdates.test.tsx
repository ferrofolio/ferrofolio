import { describe, it, expect, beforeEach } from "vitest"
import { act, cleanup } from "@testing-library/react"
import {
  mockRunTrackedUpdatesIfNeeded,
  renderProvider,
  rerenderProvider,
  resetAllMocks,
} from "./setup"

beforeEach(() => {
  cleanup()
  resetAllMocks()
})

describe("tracked updates", () => {
  it("runs once the provider mounts", async () => {
    await act(async () => {
      renderProvider()
    })

    expect(mockRunTrackedUpdatesIfNeeded).toHaveBeenCalledTimes(1)
  })

  it("only runs once across rerenders", async () => {
    await act(async () => {
      renderProvider()
    })
    await act(async () => {
      rerenderProvider()
    })

    expect(mockRunTrackedUpdatesIfNeeded).toHaveBeenCalledTimes(1)
  })
})
