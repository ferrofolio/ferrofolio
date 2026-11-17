import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { MemoryRouter } from "react-router-dom"
import { ProvidersTab } from "@/components/integrations/ProvidersTab"
import en from "@/i18n/locales/en.json"
import {
  ExternalIntegrationStatus,
  ExternalIntegrationType,
  type ExternalIntegration,
} from "@/types"

const { state, checkStatus } = vi.hoisted(() => ({
  state: { app: {} as Record<string, unknown> },
  checkStatus: vi.fn(),
}))

vi.mock("@/i18n", () => ({ useI18n: () => ({ t: en, locale: "en-US" }) }))
vi.mock("@/context/AppContext", () => ({ useAppContext: () => state.app }))
vi.mock("@/components/ai/AIModelsCard", () => ({ AIModelsCard: () => null }))
vi.mock("@/hooks/useModalBackHandler", () => ({ useModalBackHandler: vi.fn() }))
vi.mock("@/services/api", async importOriginal => ({
  ...(await importOriginal<typeof import("@/services/api")>()),
  checkStatus,
}))

const sheets: ExternalIntegration = {
  id: "GOOGLE_SHEETS",
  name: "Google Sheets",
  type: ExternalIntegrationType.DATA_SOURCE,
  status: ExternalIntegrationStatus.OFF,
  available: true,
  payload_schema: { client_id: "Client ID", client_secret: "Client secret" },
}

const renderWithPublicUrl = async (publicUrl: string | null | undefined) => {
  checkStatus.mockResolvedValue({ server: { publicUrl } })
  state.app = {
    externalIntegrations: [sheets],
    fetchExternalIntegrations: vi.fn(),
    showToast: vi.fn(),
  }
  render(
    <MemoryRouter initialEntries={["/entities?focus=GOOGLE_SHEETS"]}>
      <ProvidersTab />
    </MemoryRouter>,
  )
  await vi.waitFor(() => expect(checkStatus).toHaveBeenCalled())
  await Promise.resolve()
  fireEvent.click(
    await screen.findByRole("button", {
      name: en.settings.integration.GOOGLE_SHEETS.help,
    }),
  )
  await screen.findByText(en.settings.integration.GOOGLE_SHEETS.hint.title)
}

const callbackButtons = () =>
  screen.queryAllByRole("button", { name: /\/oauth\/callback/ })

beforeEach(() => {
  vi.clearAllMocks()
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe("ProvidersTab callback hint", () => {
  it.each([
    ["missing", undefined],
    ["null", null],
    ["blank", "   "],
  ])("shows no callback address when the public URL is %s", async (_, url) => {
    await renderWithPublicUrl(url)

    expect(callbackButtons()).toHaveLength(0)
    expect(screen.getByText("…")).toBeInTheDocument()
  })

  it("uses the trimmed public URL", async () => {
    await renderWithPublicUrl("  https://x.example/  ")

    expect(
      await screen.findByRole("button", {
        name: "https://x.example/oauth/callback",
      }),
    ).toBeInTheDocument()
    expect(screen.queryByText("…")).not.toBeInTheDocument()
  })
})
