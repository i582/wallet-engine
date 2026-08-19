import {expect, test} from "@playwright/test"

import {WebScenarioRunner} from "../dsl/web-runner"
import {scenario, wallet, walletUi} from "../dsl/scenario"

const COLLECTION_URL = /#\/collectibles$/u
const NFT_URL = /#\/collectibles\/[^/]+$/u
const nftNavigationSetup = scenario("open a wallet with collectibles")
  .given(wallet().open())
  .when(wallet().create())
  .when(wallet().acceptRecovery())
  .expect(walletUi().dashboard())
  .build()

test("opens the collectible inventory and NFT detail pages", async ({page}, testInfo) => {
  await new WebScenarioRunner({page, testInfo}).run(nftNavigationSetup)

  await page.getByRole("link", {name: "View all"}).click()
  await expect(page).toHaveURL(COLLECTION_URL)
  await expect(page.getByRole("heading", {name: "Collectibles"})).toBeVisible()
  await expect(page.getByRole("link", {name: "Open Aurora Relay"})).toBeVisible()

  await page.getByRole("link", {name: "Open Aurora Relay"}).click()
  await expect(page).toHaveURL(NFT_URL)
  await expect(page.getByRole("heading", {name: "Aurora Relay"})).toBeVisible()
  await expect(page.getByRole("img", {name: "Aurora Relay artwork"})).toBeVisible()
  await expect(
    page.getByText("A deterministic collectible used by the wallet example."),
  ).toBeVisible()
})
