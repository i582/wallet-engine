import {expect, type Locator, type Page} from "@playwright/test"

const READY_HEADING: RegExp = /Create wallet|Back up your wallet|Recent activity/

export interface WebActivityObservation {
  readonly amounts: readonly string[]
  readonly directions: readonly string[]
  readonly ids: readonly string[]
}

/** Drives only user-visible controls in the Web wallet example. */
export class WebWalletDriver {
  readonly page: Page

  /** Uses one Playwright page for all wallet actions and assertions in a scenario. */
  constructor(page: Page) {
    this.page = page
  }

  /** Opens the wallet and waits until persisted browser state has been restored. */
  async open(): Promise<void> {
    await this.page.goto("/")
    await expect(this.page.getByRole("heading", {name: READY_HEADING}).first()).toBeVisible()
  }

  /** Creates a wallet through the first-launch screen. */
  async create(): Promise<void> {
    await this.page.getByRole("button", {name: "Create wallet"}).click()
    await expect(this.page.getByRole("heading", {name: "Back up your wallet"})).toBeVisible()
  }

  /** Confirms that the recovery phrase was saved and opens the dashboard. */
  async acceptRecovery(): Promise<void> {
    await this.page.getByLabel("I saved these words.").check()
    await this.page.getByRole("button", {name: "Continue"}).click()
    await expect(this.page.getByRole("heading", {name: "Recent activity"})).toBeVisible()
  }

  /** Reloads the current page and waits for the wallet dashboard to restore. */
  async reloadDashboard(): Promise<void> {
    await this.page.reload()
    await expect(this.page.getByRole("heading", {name: "Recent activity"})).toBeVisible()
  }

  /** Refreshes account and activity data and waits for both provider requests to finish. */
  async refresh(): Promise<void> {
    const activityResponse = this.page.waitForResponse(
      response => response.url().includes("/api/v2/getTransactions") && response.ok(),
    )
    const refresh = this.page.getByRole("button", {name: "Refresh"})
    await refresh.click()
    await activityResponse
    await expect(refresh).toBeEnabled()
  }

  /** Pastes a dApp connection URL into the wallet's TON Connect dialog. */
  async openConnection(link: string): Promise<void> {
    await this.page.getByRole("button", {name: "Connect", exact: true}).click()
    await expect(this.dialog()).toBeVisible()
    await this.page.getByLabel("TON Connect link").fill(link)
    await this.dialog().getByRole("button", {name: "Continue"}).click()
  }

  /** Opens the TON Connect panel from either disconnected or connected dashboard state. */
  async openTonConnectPanel(): Promise<void> {
    if (await this.dialog().isVisible()) {
      return
    }
    const trigger: Locator = this.page.getByRole("button", {name: /^(Connect|Connected)$/})
    await expect(trigger).toBeVisible()
    await trigger.click()
    await expect(this.dialog()).toBeVisible()
  }

  /** Closes the informational dialog currently shown by the wallet. */
  async closeDialog(): Promise<void> {
    await this.dialog().getByRole("button", {name: "Close"}).click()
    await expect(this.dialog()).toBeHidden()
  }

  /** Approves the connect request currently shown by the wallet. */
  async approveConnection(): Promise<void> {
    await this.dialog().getByRole("button", {name: "Connect"}).click()
    await expect(this.dialog()).toBeHidden()
  }

  /** Approves the transaction request currently shown by the wallet. */
  async approveRequest(): Promise<void> {
    await this.dialog().getByRole("button", {name: "Confirm"}).click()
    await expect(this.dialog()).toBeHidden()
  }

  /** Rejects the connection request currently shown by the wallet. */
  async rejectConnection(): Promise<void> {
    await this.dialog().getByRole("button", {name: "Cancel"}).click()
    await expect(
      this.dialog().getByRole("heading", {name: "Paste a connection link"}),
    ).toBeVisible()
  }

  /** Rejects the transaction or signature request currently shown by the wallet. */
  async rejectRequest(): Promise<void> {
    await this.dialog().getByRole("button", {name: "Cancel"}).click()
    await expect(this.dialog()).toBeHidden()
  }

  /** Reads stable activity identifiers and semantic row values in visual order. */
  async observeActivity(count: number): Promise<WebActivityObservation> {
    const rows: Locator = this.page.locator("[data-activity-id]")
    await expect(rows).toHaveCount(count)
    const values = await rows.evaluateAll(elements =>
      elements.map(element => ({
        amount: element.getAttribute("data-activity-amount") ?? "",
        direction: element.getAttribute("data-activity-direction") ?? "",
        id: element.getAttribute("data-activity-id") ?? "",
      })),
    )
    return {
      amounts: values.map(value => value.amount),
      directions: values.map(value => value.direction),
      ids: values.map(value => value.id),
    }
  }

  /** Returns the active wallet dialog for scoped actions and assertions. */
  dialog(): Locator {
    return this.page.getByRole("dialog")
  }

  /** Returns the recovery-word grid, which contains secret and random text. */
  recoveryWords(): Locator {
    return this.page.locator("ol")
  }

  /** Returns the compact address control shown in the dashboard header. */
  walletAddress(): Locator {
    return this.page.getByRole("button", {name: "Copy wallet address"})
  }
}
