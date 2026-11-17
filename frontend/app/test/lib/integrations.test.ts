import { describe, expect, it } from "vitest"
import en from "@/i18n/locales/en.json"
import es from "@/i18n/locales/es.json"
import itTranslations from "@/i18n/locales/it.json"
import de from "@/i18n/locales/de.json"
import {
  buildExternalEntityCallbackUrl,
  buildIntegrationCallbackUrl,
  EXTERNAL_ENTITY_CALLBACK_PLACEHOLDER,
  getIntegrationFamilyName,
  getIntegrationMethodName,
  getIntegrationName,
  INTEGRATION_CALLBACK_PLACEHOLDER,
  resolveIntegrationHintPlaceholders,
  resolveIntegrationHintUrl,
} from "@/lib/integrations"

describe("integration display names", () => {
  it.each([
    [en, ["Custom AI", "Local models", "Claude Agent"]],
    [es, ["IA personalizada", "Modelos locales", "Agente de Claude"]],
    [
      itTranslations,
      ["IA personalizzata", "Modelli locali", "Agente di Claude"],
    ],
    [de, ["Eigene KI", "Lokale Modelle", "Claude Agent"]],
  ] as const)(
    "resolves each locale's descriptive names without metadata",
    (translations, names) => {
      const ids = ["OPENAI_COMPATIBLE", "LOCAL_MODELS", "CLAUDE_AGENT"]
      ids.forEach((id, index) => {
        expect(getIntegrationName(translations, id)).toBe(names[index])
        expect(
          getIntegrationName(translations, id, { id, name: "My provider" }),
        ).toBe(names[index])
      })
      expect(getIntegrationName(translations, "GOOGLE_SHEETS")).toBe(
        "Google Sheets",
      )
      expect(getIntegrationName(translations, "ENABLE_BANKING")).toBe(
        "Enable Banking",
      )
      expect(
        Object.entries(translations.settings.integration)
          .filter(([, copy]) => "name" in copy)
          .map(([id]) => id)
          .sort(),
      ).toEqual([
        "CLAUDE_AGENT",
        "ENABLE_BANKING",
        "GOOGLE_SHEETS",
        "LOCAL_MODELS",
        "OPENAI_COMPATIBLE",
      ])
    },
  )

  it.each([en, es, itTranslations, de])(
    "uses backend brand names without requiring translation entries",
    translations => {
      const integration = Object.freeze({ id: "OPENAI", name: "OpenAI" })
      expect(
        getIntegrationName(translations, integration.id, integration),
      ).toBe("OpenAI")
      expect(integration).toEqual({ id: "OPENAI", name: "OpenAI" })
    },
  )

  it("prefers optional translated names over backend names", () => {
    const translations = {
      ...en,
      settings: {
        ...en.settings,
        integration: {
          ...en.settings.integration,
          LOCAL_MODELS: {
            ...en.settings.integration.LOCAL_MODELS,
            name: "Translated models",
          },
        },
      },
    }
    expect(
      getIntegrationName(translations, "LOCAL_MODELS", {
        id: "LOCAL_MODELS",
        name: "Manually edited name",
      }),
    ).toBe("Translated models")
    expect(getIntegrationName(translations, "LOCAL_MODELS")).toBe(
      "Translated models",
    )
  })

  it("supports unknown integration IDs and missing or empty metadata", () => {
    expect(
      getIntegrationName(en, "FUTURE_API", {
        id: "FUTURE_API",
        name: "Future API",
      }),
    ).toBe("Future API")
    expect(getIntegrationName(en, "FUTURE_API")).toBe("FUTURE_API")
    expect(
      getIntegrationName(en, "FUTURE_API", { id: "FUTURE_API", name: " " }),
    ).toBe("FUTURE_API")
  })

  it("keeps localized family and method labels distinct", () => {
    expect(
      getIntegrationFamilyName(es, {
        id: "LOCAL_MODELS",
        name: "Local models",
      }),
    ).toBe("Modelos locales")
    expect(getIntegrationMethodName(es, "LOCAL_MODELS")).toBe(
      "Entorno local o servidor",
    )
    expect(getIntegrationMethodName(itTranslations, "CHATGPT")).toBe(
      "Piano ChatGPT",
    )
  })

  it("supports unknown families and methods without a frontend registry", () => {
    expect(
      getIntegrationFamilyName(en, {
        id: "FUTURE_FAMILY",
        name: "Future Service",
      }),
    ).toBe("Future Service")
    expect(
      getIntegrationFamilyName(en, { id: "FUTURE_FAMILY", name: "" }),
    ).toBe("FUTURE_FAMILY")
    expect(
      getIntegrationMethodName(en, "FUTURE_API", {
        id: "FUTURE_API",
        name: "Future API",
      }),
    ).toBe("Future API")
    expect(getIntegrationMethodName(en, "FUTURE_API")).toBe("FUTURE_API")
  })
})

describe("external entity callback URL", () => {
  const origin = "https://app.example.com"

  it("is built from an absolute server address", () => {
    expect(
      buildExternalEntityCallbackUrl("https://finance.example.com", origin),
    ).toBe("https://finance.example.com/api/v1/entities/external/callback")
  })

  it("ignores trailing slashes of the server address", () => {
    expect(
      buildExternalEntityCallbackUrl("http://localhost:7592//", origin),
    ).toBe("http://localhost:7592/api/v1/entities/external/callback")
  })

  it("falls back to the page origin for a relative server address", () => {
    expect(buildExternalEntityCallbackUrl("", origin)).toBe(
      "https://app.example.com/api/v1/entities/external/callback",
    )
  })

  it("replaces the placeholder once the callback URL is known", () => {
    const callbackUrl = buildExternalEntityCallbackUrl(
      "https://finance.example.com",
      origin,
    )

    expect(
      resolveIntegrationHintUrl(
        EXTERNAL_ENTITY_CALLBACK_PLACEHOLDER,
        callbackUrl,
      ),
    ).toBe(callbackUrl)
  })

  it("leaves other URLs untouched", () => {
    expect(resolveIntegrationHintUrl("https://example.com/", null)).toBe(
      "https://example.com/",
    )
  })

  it("does not resolve the placeholder while the server address is unknown", () => {
    expect(
      resolveIntegrationHintUrl(EXTERNAL_ENTITY_CALLBACK_PLACEHOLDER, null),
    ).toBeNull()
  })

  it.each([
    ["en", en],
    ["es", es],
    ["it", itTranslations],
    ["de", de],
  ])("no longer points %s users to the upstream relay", (_, locale) => {
    const text = JSON.stringify(locale)

    expect(text).not.toContain("finanze.me/eb/")
    expect(text).toContain(EXTERNAL_ENTITY_CALLBACK_PLACEHOLDER)
  })
})

describe("integration callback URL", () => {
  const origin = "https://app.example.com"

  it("is built from an absolute server address", () => {
    expect(
      buildIntegrationCallbackUrl("https://finance.example.com/", origin),
    ).toBe("https://finance.example.com/oauth/callback")
  })

  it("falls back to the page origin for a relative server address", () => {
    expect(buildIntegrationCallbackUrl("", origin)).toBe(
      "https://app.example.com/oauth/callback",
    )
  })

  it("resolves every known placeholder", () => {
    const urls = {
      [EXTERNAL_ENTITY_CALLBACK_PLACEHOLDER]: "https://a.example/eb",
      [INTEGRATION_CALLBACK_PLACEHOLDER]: "https://a.example/oauth",
    }

    expect(
      resolveIntegrationHintPlaceholders(
        INTEGRATION_CALLBACK_PLACEHOLDER,
        urls,
      ),
    ).toBe("https://a.example/oauth")
    expect(
      resolveIntegrationHintPlaceholders(
        EXTERNAL_ENTITY_CALLBACK_PLACEHOLDER,
        urls,
      ),
    ).toBe("https://a.example/eb")
  })

  it("does not resolve a placeholder whose URL is unknown", () => {
    expect(
      resolveIntegrationHintPlaceholders(INTEGRATION_CALLBACK_PLACEHOLDER, {
        [INTEGRATION_CALLBACK_PLACEHOLDER]: null,
      }),
    ).toBeNull()
  })

  it.each([
    ["en", en],
    ["es", es],
    ["it", itTranslations],
    ["de", de],
  ])("no longer points %s users to the upstream OAuth relay", (_, locale) => {
    const text = JSON.stringify(locale)

    expect(text).not.toContain("finanze.me/oauth/")
    expect(text).toContain(INTEGRATION_CALLBACK_PLACEHOLDER)
  })
})
