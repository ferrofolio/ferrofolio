export const INTEGRATION_OAUTH_PARAM = "integration_oauth"

export interface IntegrationOAuthCallback {
  integrationId: string
  state: string | null
  code: string | null
  error: string | null
  clientId: string | null
}

export async function openExternalAuthLink(link: string) {
  // Navigate the same tab; the provider page redirects back to this app with
  // the code/state once the authorization completes.
  window.location.assign(link)
}

export async function openExternalAuthPage(link: string) {
  try {
    window.open(link, "_blank", "noopener,noreferrer")
  } catch {
    // ignore window open errors
  }
}

export function buildIntegrationCompletionUrl(integrationId: string): string {
  const query = `${INTEGRATION_OAUTH_PARAM}=${encodeURIComponent(integrationId)}`
  const route = `#/entities?tab=providers&focus=${encodeURIComponent(integrationId)}`
  return `${window.location.origin}${window.location.pathname}?${query}${route}`
}

export function parseIntegrationOAuthCallback(
  search: string,
): IntegrationOAuthCallback | null {
  const params = new URLSearchParams(search)
  const integrationId = params.get(INTEGRATION_OAUTH_PARAM)
  if (!integrationId) return null
  return {
    integrationId,
    state: params.get("state"),
    code: params.get("code"),
    error: params.get("error"),
    clientId: params.get("client_id"),
  }
}

export function parseOAuthCallbackAddress(
  integrationId: string,
  address: string,
): IntegrationOAuthCallback | null {
  let url: URL
  try {
    url = new URL(address.trim())
  } catch {
    return null
  }
  const state = url.searchParams.get("state")
  if (!state) return null
  return {
    integrationId,
    state,
    code: url.searchParams.get("code"),
    error: url.searchParams.get("error"),
    clientId: url.searchParams.get("client_id"),
  }
}
