import { expect } from '@playwright/test'
import { test } from '../../fixtures/auth'

test.describe('Disconnect Entity (E2E Invest)', () => {
    test('disconnect entity removes it from connected list', async ({
        authenticatedPage: page,
    }) => {
        // Navigate to entities
        await page.getByRole('button', { name: 'Integrations' }).click()
        await page
            .getByRole('heading', { name: 'Integrations' })
            .waitFor({ timeout: 15_000 })

        const investCard = page
            .locator('h3', { hasText: 'E2E Invest' })
            .first()
            .locator('../..')
        const fetchButton = investCard.getByRole('button', {
            name: 'Fetch',
        })
        const isAlreadyConnected = await fetchButton
            .isVisible({ timeout: 2_000 })
            .catch(() => false)

        if (!isAlreadyConnected) {
            // Connect E2E Invest first
            await page.getByText('E2E Invest').first().click()
            await page
                .getByText('Enter credentials for')
                .waitFor({ timeout: 5_000 })
            await page.locator('#user').fill('test@example.com')
            await page.locator('#password').fill('MockPassword123')
            await page.getByRole('button', { name: 'Submit' }).click()
            await expect(
                page.getByText('Successfully logged in to E2E Invest'),
            ).toBeVisible({ timeout: 15_000 })

            // Reload to ensure entity accounts are populated from the backend
            await page.reload()
            await page.waitForLoadState('networkidle')

            // Navigate to Integrations
            await page.getByRole('button', { name: 'Integrations' }).click()
            await page
                .getByRole('heading', { name: 'Integrations' })
                .waitFor({ timeout: 15_000 })
        }

        // Verify the entity exposes its connected-state action
        await expect(fetchButton).toBeVisible({
            timeout: 5_000,
        })

        // Click the disconnect button (red Unplug icon) on E2E Invest card
        const disconnectButton = investCard.locator(
            'button.text-red-600, button.text-red-500',
        )
        await disconnectButton.click()

        // Confirmation dialog should appear
        await expect(page.getByText('Confirm Disconnect')).toBeVisible({
            timeout: 5_000,
        })

        // Click "Disconnect" to confirm
        await page.getByRole('button', { name: 'Disconnect' }).click()

        // Success toast should appear
        await expect(
            page.getByText('Entity disconnected successfully'),
        ).toBeVisible({ timeout: 10_000 })

        // E2E Invest should now be in "Available Entities" (disconnected state)
        await expect(page.getByText('Available Entities')).toBeVisible({
            timeout: 5_000,
        })
    })
})
