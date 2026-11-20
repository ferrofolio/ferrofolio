import { defineConfig, devices } from '@playwright/test'

const BACKEND_PORT = Number(process.env.E2E_BACKEND_PORT || 7692)
const FRONTEND_PORT = Number(process.env.E2E_FRONTEND_PORT || 5273)

export default defineConfig<{}, { backendPort: number }>({
    testDir: './tests',
    timeout: 60_000,
    retries: 1,
    workers: 1,
    fullyParallel: false,

    use: {
        baseURL: `http://localhost:${FRONTEND_PORT}`,
        trace: 'on-first-retry',
        screenshot: 'only-on-failure',
    },

    projects: [
        {
            name: 'setup',
            testMatch: /signup\.spec\.ts/,
            use: { ...devices['Desktop Chrome'] },
        },
        {
            name: 'main',
            testIgnore: /signup\.spec\.ts/,
            dependencies: ['setup'],
            use: { ...devices['Desktop Chrome'] },
        },
    ],

    webServer: [
        {
            command: `cd ../../frontend/app && VITE_BASE_URL=http://localhost:${BACKEND_PORT} VITE_E2E_MOCK_LOGIN=1 pnpm dev --port ${FRONTEND_PORT}`,
            port: FRONTEND_PORT,
            reuseExistingServer: !process.env.CI,
            timeout: 30_000,
        },
    ],
})
