import { expect, type Page } from '@playwright/test'
import { test } from '../../fixtures/auth'
import { selectEntity } from '../../helpers/entity-selector'
import { navigateToMyMoneyPage } from '../../helpers/my-money'

/**
 * Helper: connect an entity via login form and reload.
 */
async function connectEntityIfNeeded(
    page: Page,
    entityName: string,
    credentials: Record<string, string>,
) {
    await page.getByRole('button', { name: 'Integrations' }).click()
    await page
        .getByRole('heading', { name: 'Integrations' })
        .waitFor({ timeout: 15_000 })

    // Dismiss any overlay
    const cancelBtn = page.getByRole('button', { name: 'Cancel' })
    if (await cancelBtn.isVisible({ timeout: 1_000 }).catch(() => false)) {
        await cancelBtn.click()
        await page.waitForTimeout(500)
    }

    const entityCard = page
        .locator('h3', { hasText: entityName })
        .first()
        .locator('../..')
    const fetchBtn = entityCard.getByRole('button', { name: 'Fetch' })
    const isConnected = await fetchBtn
        .isVisible({ timeout: 3_000 })
        .catch(() => false)

    if (!isConnected) {
        await page.getByText(entityName).first().click()
        await page
            .getByText('Enter credentials for')
            .waitFor({ timeout: 10_000 })
        for (const [field, value] of Object.entries(credentials)) {
            await page.locator(`#${field}`).fill(value)
        }
        await page.getByRole('button', { name: 'Submit' }).click()
        await expect(
            page.getByText(`Successfully logged in to ${entityName}`),
        ).toBeVisible({ timeout: 30_000 })

        await page.reload()
        await page.waitForLoadState('networkidle')

        await page.getByRole('button', { name: 'Integrations' }).click()
        await page
            .getByRole('heading', { name: 'Integrations' })
            .waitFor({ timeout: 15_000 })
    }
}

/**
 * Helper: fetch all features for an entity from Integrations page.
 * Waits for features to be selected, clicks Fetch, waits for success, then closes overlay.
 */
async function fetchEntityAllFeatures(page: Page, entityName: string) {
    const entityCard = page
        .locator('h3', { hasText: entityName })
        .first()
        .locator('../..')
    await entityCard.getByRole('button', { name: 'Fetch' }).click()

    await expect(
        page.getByText(`Select features to fetch from ${entityName}`),
    ).toBeVisible({ timeout: 5_000 })

    // Wait for features to be auto-selected (Fetch button becomes enabled)
    const modalFetch = page
        .locator('.fixed')
        .getByRole('button', { name: 'Fetch' })
    await expect(modalFetch).toBeEnabled({
        timeout: 5_000,
    })

    await modalFetch.click()

    await expect(
        page.getByText(`Data successfully fetched from ${entityName}`),
    ).toBeVisible({ timeout: 30_000 })

    // Close the FeatureSelector overlay by clicking Cancel. Scope the locator to
    // the overlay and tolerate the click failing if the overlay is already
    // animating out (the element can detach mid-click), then confirm it's gone.
    const cancelBtn = page
        .locator('.fixed')
        .getByRole('button', { name: 'Cancel' })
    await cancelBtn.click({ timeout: 5_000 }).catch(() => {})
    await expect(
        page.getByText(`Select features to fetch from ${entityName}`),
    ).toBeHidden({ timeout: 5_000 })
}

async function navigateToAutoContributions(page: Page) {
    await navigateToMyMoneyPage(page, 'Budget')
    await page.getByRole('tab', { name: /^Investments/ }).click()
}

// E2E Invest: has POSITION, TRANSACTIONS, HISTORIC (no AUTO_CONTRIBUTIONS)
// E2E Broker: has POSITION, AUTO_CONTRIBUTIONS, TRANSACTIONS

test.describe('Fetch Entity Data - Transactions', () => {
    test('fetched transactions appear in TransactionsPage', async ({
        authenticatedPage: page,
    }) => {
        await connectEntityIfNeeded(page, 'E2E Invest', {
            user: 'test@example.com',
            password: 'MockPassword123',
        })
        await fetchEntityAllFeatures(page, 'E2E Invest')

        // Navigate to Transactions page
        await page
            .getByRole('navigation')
            .getByRole('button', { name: 'Transactions' })
            .click()
        await page
            .getByRole('heading', { name: 'Transactions' })
            .first()
            .waitFor({ timeout: 10_000 })
        await page
            .getByRole('radio', { name: 'Investments', exact: true })
            .click()
        await selectEntity(page, 'E2E Invest')
        await page.keyboard.press('Escape')
        await page.getByRole('button', { name: 'Search', exact: true }).click()

        // Verify mock transactions are visible
        await expect(page.getByText('Mock Stock A').first()).toBeVisible({
            timeout: 5_000,
        })
        await expect(page.getByText('Mock Stock B').first()).toBeVisible({
            timeout: 5_000,
        })
    })
})

test.describe('Fetch Entity Data - Auto Contributions', () => {
    test('fetched auto-contributions appear in AutoContributionsPage', async ({
        authenticatedPage: page,
    }) => {
        // E2E Broker has AUTO_CONTRIBUTIONS feature
        await connectEntityIfNeeded(page, 'E2E Broker', {
            user: 'test@example.com',
            password: 'MockPassword123',
        })
        await fetchEntityAllFeatures(page, 'E2E Broker')

        await navigateToAutoContributions(page)

        // Verify mock contribution is visible
        await expect(page.getByText('Mock ETF Plan').first()).toBeVisible({
            timeout: 5_000,
        })
    })
})

test.describe('Fetch Entity Data - Disconnect Cleanup', () => {
    test('disconnect removes transactions and contributions', async ({
        authenticatedPage: page,
    }) => {
        // Connect and fetch E2E Broker (has both TRANSACTIONS and AUTO_CONTRIBUTIONS)
        await connectEntityIfNeeded(page, 'E2E Broker', {
            user: 'test@example.com',
            password: 'MockPassword123',
        })
        await fetchEntityAllFeatures(page, 'E2E Broker')

        // Verify data exists before disconnect
        await navigateToAutoContributions(page)
        await expect(page.getByText('Mock ETF Plan').first()).toBeVisible({
            timeout: 5_000,
        })

        // Disconnect E2E Broker
        await page.getByRole('button', { name: 'Integrations' }).click()
        await page
            .getByRole('heading', { name: 'Integrations' })
            .waitFor({ timeout: 15_000 })

        const entityCard = page
            .locator('h3', { hasText: 'E2E Broker' })
            .first()
            .locator('../..')
        const disconnectButton = entityCard.locator(
            'button.text-red-600, button.text-red-500',
        )
        await disconnectButton.click()

        await expect(page.getByText('Confirm Disconnect')).toBeVisible({
            timeout: 5_000,
        })
        await page.getByRole('button', { name: 'Disconnect' }).click()
        await expect(
            page.getByText('Entity disconnected successfully'),
        ).toBeVisible({ timeout: 10_000 })

        // Disconnect all other entities that may be connected from prior specs in same worker
        for (const otherEntity of [
            'E2E Invest',
            'Trade Republic',
            'ING',
            'Unicaja',
        ]) {
            await page.reload()
            await page.waitForLoadState('networkidle')
            await page.getByRole('button', { name: 'Integrations' }).click()
            await page
                .getByRole('heading', { name: 'Integrations' })
                .waitFor({ timeout: 15_000 })

            const otherCard = page
                .locator('h3', { hasText: otherEntity })
                .first()
                .locator('../..')
            const otherDisconnect = otherCard.locator(
                'button.text-red-600, button.text-red-500',
            )
            if (
                await otherDisconnect
                    .isVisible({ timeout: 3_000 })
                    .catch(() => false)
            ) {
                await otherDisconnect.click()
                await expect(page.getByText('Confirm Disconnect')).toBeVisible({
                    timeout: 5_000,
                })
                await page.getByRole('button', { name: 'Disconnect' }).click()
                await expect(
                    page.getByText('Entity disconnected successfully'),
                ).toBeVisible({ timeout: 10_000 })
            }
        }

        // Verify E2E Broker and E2E Invest transactions are gone
        await page
            .getByRole('navigation')
            .getByRole('button', { name: 'Transactions' })
            .click()
        await page
            .getByRole('heading', { name: 'Transactions' })
            .first()
            .waitFor({ timeout: 10_000 })
        await expect(page.getByText('Mock Stock A').first()).not.toBeVisible()
        await expect(page.getByText('Mock Stock B').first()).not.toBeVisible()

        // Verify contributions are gone
        await navigateToAutoContributions(page)
        await expect(
            page
                .getByText('No auto contributions')
                .filter({ visible: true }),
        ).toBeVisible({ timeout: 5_000 })
    })
})
