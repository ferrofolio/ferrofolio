export interface LoginHandlerResult {
  success: boolean
  credentials: Record<string, string>
  flow?: "login" | "fetch"
}

export interface ExternalLoginAPI {
  requestExternalLogin(
    id: string,
    request?: {
      credentials?: Record<string, string>
      flow?: "login" | "fetch"
    },
  ): Promise<{ success: boolean }>
  onCompletedExternalLogin(
    callback: (id: string, result: LoginHandlerResult) => void,
  ): () => void
}

// No external login window is available in the browser. Only the end-to-end
// tests register an implementation.
let externalLoginAPI: ExternalLoginAPI | null = null

export function getExternalLoginAPI(): ExternalLoginAPI | null {
  return externalLoginAPI
}

export function setExternalLoginAPI(api: ExternalLoginAPI | null) {
  externalLoginAPI = api
}
