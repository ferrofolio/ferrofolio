import type { ExternalIntegration, ExternalIntegrationFamily } from "@/types"

type IntegrationName = Pick<ExternalIntegration, "id" | "name">
type IntegrationTranslations = {
  settings: {
    integration: Record<string, { name?: string; description?: string }>
    aiProviderFamilies: Record<string, string>
    aiProviderMethods: Record<string, string>
  }
}

export function getIntegrationName(
  translations: IntegrationTranslations,
  integrationId: string,
  integration?: IntegrationName,
) {
  const copy = translations.settings.integration[integrationId]
  return copy?.name?.trim() || integration?.name.trim() || integrationId
}

export function getIntegrationFamilyName(
  translations: IntegrationTranslations,
  family: ExternalIntegrationFamily,
) {
  return (
    translations.settings.aiProviderFamilies[family.id]?.trim() ||
    family.name.trim() ||
    family.id
  )
}

export function getIntegrationMethodName(
  translations: IntegrationTranslations,
  integrationId: string,
  integration?: IntegrationName,
) {
  return (
    translations.settings.aiProviderMethods[integrationId]?.trim() ||
    getIntegrationName(translations, integrationId, integration)
  )
}

export const EXTERNAL_ENTITY_CALLBACK_PATH =
  "/api/v1/entities/external/callback"
export const EXTERNAL_ENTITY_CALLBACK_PLACEHOLDER =
  "{{externalEntityCallbackUrl}}"

/**
 * URL of this server that an account-information provider redirects to after the
 * user gave consent. It has to be registered with the provider, so it is built
 * from the address the app uses to reach the server.
 */
export function buildExternalEntityCallbackUrl(
  apiBaseUrl: string,
  origin: string,
): string {
  const base = apiBaseUrl.replace(/\/+$/, "")
  return new URL(`${base}${EXTERNAL_ENTITY_CALLBACK_PATH}`, origin).toString()
}

export const INTEGRATION_CALLBACK_PATH = "/oauth/callback"
export const INTEGRATION_CALLBACK_PLACEHOLDER = "{{integrationCallbackUrl}}"

/**
 * URL of this server that an OAuth provider such as Google redirects to after
 * the user authorized an integration. It has to be registered with the provider.
 */
export function buildIntegrationCallbackUrl(
  serverUrl: string,
  origin: string,
): string {
  const base = serverUrl.replace(/\/+$/, "")
  return new URL(`${base}${INTEGRATION_CALLBACK_PATH}`, origin).toString()
}

export type IntegrationHintUrls = Record<string, string | null>

/**
 * Replace the callback placeholders used in integration setup instructions.
 * While the server address is not known yet a placeholder resolves to null.
 */
export function resolveIntegrationHintPlaceholders(
  value: string,
  urls: IntegrationHintUrls,
): string | null {
  let resolved = value
  for (const [placeholder, url] of Object.entries(urls)) {
    if (!resolved.includes(placeholder)) continue
    if (!url) return null
    resolved = resolved.split(placeholder).join(url)
  }
  return resolved
}

export function resolveIntegrationHintUrl(
  value: string,
  callbackUrl: string | null,
): string | null {
  return resolveIntegrationHintPlaceholders(value, {
    [EXTERNAL_ENTITY_CALLBACK_PLACEHOLDER]: callbackUrl,
  })
}
