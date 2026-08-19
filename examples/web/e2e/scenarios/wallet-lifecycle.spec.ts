import {test} from "@playwright/test"

import {WebScenarioRunner} from "../dsl/web-runner"
import {walletLifecycleScenario} from "./definitions"

test("creates and restores a wallet", async ({page}, testInfo) => {
  await new WebScenarioRunner({page, testInfo}).run(walletLifecycleScenario)
})
