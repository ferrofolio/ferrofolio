export interface ChallengeWindowAPI {
  requestChallengeWindow(
    siteKey: string,
    domain: string,
  ): Promise<{ success: boolean }>
  onChallengeCompleted(callback: (token: string | null) => void): () => void
}

// No challenge window is available in the browser. Only the end-to-end tests
// register an implementation.
let challengeWindowAPI: ChallengeWindowAPI | null = null

export function getChallengeWindowAPI(): ChallengeWindowAPI | null {
  return challengeWindowAPI
}

export function setChallengeWindowAPI(api: ChallengeWindowAPI | null) {
  challengeWindowAPI = api
}
