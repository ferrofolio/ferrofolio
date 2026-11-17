import { afterEach, describe, expect, it, vi } from "vitest"
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"

import { RealEstateFormModal } from "@/components/RealEstateFormModal"
import { DataDisplayModeProvider } from "@/context/DataDisplayModeContext"
import { ModalRegistryProvider } from "@/context/ModalRegistryContext"
import en from "@/i18n/locales/en.json"
import type { RealEstate } from "@/types"

const mocks = vi.hoisted(() => ({
  createRealEstate: vi.fn().mockResolvedValue(undefined),
  updateRealEstate: vi.fn().mockResolvedValue(undefined),
  app: {
    settings: { general: { defaultCurrency: "EUR" } },
    showToast: vi.fn(),
  },
  financial: {
    periodicFlows: [],
    positionsData: null,
    refreshFlows: vi.fn().mockResolvedValue(undefined),
    refreshRealEstate: vi.fn().mockResolvedValue(undefined),
    ensurePeriodicFlows: vi.fn().mockResolvedValue(undefined),
  },
  i18n: { t: {} as unknown, locale: "en-US" },
}))

mocks.i18n.t = en
vi.mock("@/i18n", () => ({ useI18n: () => mocks.i18n }))
vi.mock("@/context/AppContext", () => ({
  useAppContext: () => mocks.app,
}))
vi.mock("@/context/FinancialDataContext", () => ({
  useFinancialData: () => mocks.financial,
}))
vi.mock("@/services/api", () => ({
  createRealEstate: mocks.createRealEstate,
  updateRealEstate: mocks.updateRealEstate,
  getImageUrl: vi.fn().mockResolvedValue(""),
  calculateLoan: vi.fn(),
}))

const property: RealEstate = {
  id: "re-1",
  currency: "EUR",
  basic_info: { name: "Flat", is_residence: true, is_rented: false },
  location: {},
  purchase_info: { date: "2020-01-01", price: 200000, expenses: [] },
  valuation_info: {
    estimated_market_value: 250000,
    valuations: [{ date: "2024-01-01", amount: 250000, market_value: true }],
  },
  flows: [],
  rental_data: { amortizations: [], vacancy_rate: null },
}

afterEach(cleanup)

describe("real estate photo upload", () => {
  it("previews and uploads the selected file unchanged", async () => {
    const { container } = render(
      <DataDisplayModeProvider>
        <ModalRegistryProvider>
          <RealEstateFormModal
            isOpen
            onClose={vi.fn()}
            property={property}
            onSuccess={vi.fn()}
          />
        </ModalRegistryProvider>
      </DataDisplayModeProvider>,
    )
    const photo = new File(["photo-bytes"], "house.jpg", {
      type: "image/jpeg",
    })
    const input = container.ownerDocument.getElementById(
      "photo-upload",
    ) as HTMLInputElement

    fireEvent.change(input, { target: { files: [photo] } })

    const preview = await screen.findByAltText("Preview")
    expect(preview.getAttribute("src")).toBe(
      `data:image/jpeg;base64,${btoa("photo-bytes")}`,
    )

    fireEvent.click(screen.getByRole("button", { name: en.common.save }))

    await waitFor(() => expect(mocks.updateRealEstate).toHaveBeenCalledTimes(1))
    expect(mocks.updateRealEstate.mock.calls[0][0].photo).toBe(photo)
    expect(mocks.createRealEstate).not.toHaveBeenCalled()
  })
})
