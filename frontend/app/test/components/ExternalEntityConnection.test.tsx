import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react"
import {
  ExternalEntityConnectionModals,
  useExternalEntityConnection,
} from "@/components/ExternalEntityConnection"
import en from "@/i18n/locales/en.json"
import es from "@/i18n/locales/es.json"
import {
  EntityType,
  ExternalEntitySetupResponseCode,
  ExternalIntegrationStatus,
  ExternalIntegrationType,
} from "@/types"
import {
  completeExternalEntityConnection,
  connectExternalEntity,
  getExternalEntityCandidates,
} from "@/services/api"
import { useModalBackHandler } from "@/hooks/useModalBackHandler"
import {
  ModalRegistryProvider,
  useModalRegistry,
} from "@/context/ModalRegistryContext"

const { state, app, workflow } = vi.hoisted(() => ({
  state: {
    integrations: [] as unknown[],
    locale: "en-US" as "en-US" | "es-ES",
  },
  app: {
    fetchEntities: vi.fn(),
    showToast: vi.fn(),
    fetchExternalIntegrations: vi.fn(),
  },
  workflow: { setView: vi.fn() },
}))

vi.mock("@/i18n", () => ({
  useI18n: () => ({
    t: state.locale === "es-ES" ? es : en,
    locale: state.locale,
  }),
}))
vi.mock("@/context/AppContext", () => ({
  useAppContext: () => ({
    ...app,
    externalIntegrations: state.integrations,
  }),
}))
vi.mock("@/context/EntityWorkflowContext", () => ({
  useEntityWorkflow: () => workflow,
}))
vi.mock("@/services/api", () => ({
  connectExternalEntity: vi.fn(),
  getExternalEntityCandidates: vi.fn(),
  completeExternalEntityConnection: vi.fn(),
  disconnectExternalEntity: vi.fn(),
  getImageUrl: vi.fn(),
}))
vi.mock("@/lib/externalAuth", () => ({
  INTEGRATION_OAUTH_PARAM: "integration_oauth",
  openExternalAuthLink: vi.fn(),
}))
vi.mock("@/hooks/useModalBackHandler", () => ({
  useModalBackHandler: vi.fn(),
}))

const enableBanking = {
  id: "ENABLE_BANKING",
  name: "Enable Banking",
  status: ExternalIntegrationStatus.ON,
  type: ExternalIntegrationType.ENTITY_PROVIDER,
  available: true,
}

function Harness() {
  const conn = useExternalEntityConnection()
  return (
    <>
      <button onClick={conn.openAddExternalEntity}>open</button>
      <ExternalEntityConnectionModals conn={conn} />
    </>
  )
}

function bank(name: string, psuTypes: string[] | null) {
  return {
    id: `ES:${name}`,
    name,
    bic: `${name.toUpperCase().replace(/\s/g, "")}ESMM`,
    type: EntityType.FINANCIAL_INSTITUTION,
    icon: null,
    psu_types: psuTypes,
  }
}

async function openInstitution(name: string, psuTypes: string[] | null) {
  vi.mocked(getExternalEntityCandidates).mockResolvedValue({
    entities: [bank(name, psuTypes)],
  })
  render(<Harness />)
  fireEvent.click(screen.getByRole("button", { name: "open" }))
  fireEvent.click(screen.getByText("ES"))
  fireEvent.click(await screen.findByText(name))
}

describe("ExternalEntityConnection PSU type", () => {
  beforeEach(() => {
    vi.clearAllMocks()
    state.locale = "en-US"
    state.integrations = [enableBanking]
    vi.mocked(connectExternalEntity).mockResolvedValue({
      code: ExternalEntitySetupResponseCode.ALREADY_LINKED,
    })
  })

  afterEach(() => {
    cleanup()
  })

  it("localizes the provider label without translating institution names", async () => {
    state.locale = "es-ES"
    state.integrations = [{ ...enableBanking, name: "Database name" }]
    await openInstitution("Original bank name", ["personal"])
    expect(
      screen.getByText(es.entities.via.replace("{provider}", "Enable Banking")),
    ).toBeInTheDocument()
    expect(screen.getAllByText("Original bank name").length).toBeGreaterThan(0)
    expect(screen.queryByText("Database name")).not.toBeInTheDocument()
  })

  it("expands a bank that lists both types and connects with the chosen type", async () => {
    await openInstitution("Dual Bank", ["personal", "business"])

    expect(
      screen.getByRole("group", { name: en.entities.psuTypeTitle }),
    ).toBeInTheDocument()
    fireEvent.click(
      screen.getByRole("button", { name: en.entities.psuTypeBusiness }),
    )

    await waitFor(() =>
      expect(screen.getAllByText(en.entities.psuTypeBusiness)).toHaveLength(1),
    )
    expect(screen.getByText(en.entities.psuTypeTitle)).toBeInTheDocument()
    fireEvent.click(screen.getByRole("button", { name: en.common.confirm }))

    await waitFor(() =>
      expect(connectExternalEntity).toHaveBeenCalledWith(
        expect.objectContaining({
          institution_id: "ES:Dual Bank",
          provider: "ENABLE_BANKING",
          psu_type: "business",
        }),
      ),
    )
  })

  it("connects a single-type bank without expanding it", async () => {
    await openInstitution("Business Only", ["business"])

    expect(
      screen.queryByRole("group", { name: en.entities.psuTypeTitle }),
    ).not.toBeInTheDocument()
    expect(
      await screen.findByText(en.entities.psuTypeBusiness),
    ).toBeInTheDocument()
    fireEvent.click(screen.getByRole("button", { name: en.common.confirm }))

    await waitFor(() =>
      expect(connectExternalEntity).toHaveBeenCalledWith(
        expect.objectContaining({
          institution_id: "ES:Business Only",
          psu_type: "business",
        }),
      ),
    )
  })

  it("sends no psu_type and shows no account type when the bank does not list types", async () => {
    await openInstitution("Legacy Bank", null)

    await screen.findByRole("button", { name: en.common.confirm })
    expect(screen.queryByText(en.entities.psuTypeTitle)).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole("button", { name: en.common.confirm }))

    await waitFor(() => expect(connectExternalEntity).toHaveBeenCalledTimes(1))
    const [request] = vi.mocked(connectExternalEntity).mock.calls[0]
    expect(request).not.toHaveProperty("psu_type")
  })

  it("collapses an expanded bank when its card is clicked again", async () => {
    await openInstitution("Dual Bank", ["personal", "business"])
    expect(
      screen.getByRole("group", { name: en.entities.psuTypeTitle }),
    ).toBeInTheDocument()

    fireEvent.click(screen.getByText("Dual Bank"))

    await waitFor(() =>
      expect(
        screen.queryByText(en.entities.psuTypeBusiness),
      ).not.toBeInTheDocument(),
    )
    expect(connectExternalEntity).not.toHaveBeenCalled()
  })

  it("collapses an expanded bank when another bank is selected", async () => {
    vi.mocked(getExternalEntityCandidates).mockResolvedValue({
      entities: [
        bank("Dual Bank", ["personal", "business"]),
        bank("Business Only", ["business"]),
      ],
    })
    render(<Harness />)
    fireEvent.click(screen.getByRole("button", { name: "open" }))
    fireEvent.click(screen.getByText("ES"))
    fireEvent.click(await screen.findByText("Dual Bank"))
    expect(
      screen.getByRole("group", { name: en.entities.psuTypeTitle }),
    ).toBeInTheDocument()

    fireEvent.click(screen.getByText("Business Only"))

    await waitFor(() =>
      expect(screen.getAllByText(en.entities.psuTypeBusiness)).toHaveLength(1),
    )
    expect(screen.getByText(en.entities.psuTypeTitle)).toBeInTheDocument()
  })

  it("shows the bank logo in the Enable Banking confirmation", async () => {
    vi.mocked(getExternalEntityCandidates).mockResolvedValue({
      entities: [
        {
          ...bank("Logo Bank", ["business"]),
          icon: "https://example.com/logo.png",
        },
      ],
    })
    render(<Harness />)
    fireEvent.click(screen.getByRole("button", { name: "open" }))
    fireEvent.click(screen.getByText("ES"))
    fireEvent.click(await screen.findByText("Logo Bank"))

    await waitFor(() =>
      expect(screen.getAllByRole("img", { name: "Logo Bank" })).toHaveLength(2),
    )
  })
})

describe("ExternalEntityConnection provider callback", () => {
  beforeEach(() => {
    vi.clearAllMocks()
    state.locale = "en-US"
    state.integrations = [enableBanking]
    window.history.replaceState({}, "", "/?code=auth-code&state=entity-1")
  })

  afterEach(() => {
    cleanup()
    window.history.replaceState({}, "", "/")
  })

  it("completes the connection with the code from the redirect", async () => {
    vi.mocked(completeExternalEntityConnection).mockResolvedValue(undefined)
    render(<Harness />)

    await waitFor(() =>
      expect(app.showToast).toHaveBeenCalledWith(
        en.entities.externalLinkSuccess,
        "success",
      ),
    )
    expect(completeExternalEntityConnection).toHaveBeenCalledWith(
      "entity-1",
      "auth-code",
    )
    expect(window.location.search).toBe("")
    expect(app.fetchEntities).toHaveBeenCalled()
  })

  it("retries a failed completion with the same code", async () => {
    vi.mocked(completeExternalEntityConnection)
      .mockRejectedValueOnce(new Error("network"))
      .mockResolvedValueOnce(undefined)
    render(<Harness />)

    const confirm = await screen.findByRole("button", {
      name: en.entities.confirmConnection,
    })
    expect(app.showToast).toHaveBeenCalledWith(
      en.entities.externalLinkError,
      "error",
    )
    expect(app.fetchEntities).not.toHaveBeenCalled()
    expect(window.location.search).toBe("")

    fireEvent.click(confirm)

    await waitFor(() =>
      expect(app.showToast).toHaveBeenCalledWith(
        en.entities.externalLinkSuccess,
        "success",
      ),
    )
    expect(completeExternalEntityConnection).toHaveBeenCalledTimes(2)
    expect(completeExternalEntityConnection).toHaveBeenLastCalledWith(
      "entity-1",
      "auth-code",
    )
    expect(app.fetchEntities).toHaveBeenCalledTimes(1)
    expect(
      screen.queryByRole("button", { name: en.entities.confirmConnection }),
    ).not.toBeInTheDocument()
  })

  describe("Back", () => {
    let registry: ReturnType<typeof useModalRegistry>
    function RegistryProbe() {
      registry = useModalRegistry()
      return null
    }

    beforeEach(async () => {
      const actual = await vi.importActual<
        typeof import("@/hooks/useModalBackHandler")
      >("@/hooks/useModalBackHandler")
      vi.mocked(useModalBackHandler).mockImplementation(
        actual.useModalBackHandler,
      )
    })

    afterEach(() => {
      vi.mocked(useModalBackHandler).mockReset()
    })

    const renderWithRegistry = () =>
      render(
        <ModalRegistryProvider>
          <RegistryProbe />
          <Harness />
        </ModalRegistryProvider>,
      )
    // Back dismisses the topmost registered modal; an open modal stays
    // registered until it closes, independent of its exit animation. The
    // handler is re-registered in an effect after each render, so let pending
    // effects run first, as they would before a user could press Back.
    const pressBack = async () => {
      await act(async () => {})
      act(() => void registry.dismissTop())
    }

    it("is ignored while the completion request runs", async () => {
      let finish: () => void = () => {}
      vi.mocked(completeExternalEntityConnection).mockReturnValue(
        new Promise<void>(resolve => {
          finish = resolve
        }),
      )
      renderWithRegistry()
      await screen.findByText(en.entities.completingConnection)

      await pressBack()
      expect(registry.hasOpen()).toBe(true)

      await act(async () => finish())
      await waitFor(() =>
        expect(app.showToast).toHaveBeenCalledWith(
          en.entities.externalLinkSuccess,
          "success",
        ),
      )
      expect(registry.hasOpen()).toBe(false)
    })

    it("dismisses the modal once the completion has failed", async () => {
      vi.mocked(completeExternalEntityConnection).mockRejectedValue(
        new Error("network"),
      )
      renderWithRegistry()
      await screen.findByRole("button", {
        name: en.entities.confirmConnection,
      })
      expect(registry.hasOpen()).toBe(true)

      await pressBack()
      expect(registry.hasOpen()).toBe(false)
    })
  })
})
