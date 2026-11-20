import { describe, it, expect, beforeEach } from "vitest"
import { act, cleanup } from "@testing-library/react"
import {
  mockGetPositions,
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

  it("waits until the initial positions load has settled", async () => {
    let resolvePositions: (value: unknown) => void = () => {}
    mockGetPositions.mockReturnValue(
      new Promise(resolve => {
        resolvePositions = resolve
      }),
    )

    await act(async () => {
      renderProvider()
    })

    expect(mockGetPositions).toHaveBeenCalledTimes(1)
    expect(mockRunTrackedUpdatesIfNeeded).not.toHaveBeenCalled()

    await act(async () => {
      resolvePositions({ positions: {} })
    })

    expect(mockRunTrackedUpdatesIfNeeded).toHaveBeenCalledTimes(1)
  })

  it("runs after a failed initial positions load", async () => {
    mockGetPositions.mockRejectedValue(new Error("offline"))

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
