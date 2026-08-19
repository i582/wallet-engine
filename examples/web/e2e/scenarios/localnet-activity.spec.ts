import {WebScenarioRunner} from "../dsl/web-runner"
import {test} from "../fixtures/ton-connect"
import {localnetActivityScenario} from "./definitions"

test.setTimeout(120_000)

test("updates transaction history from Acton localnet", async ({page, tonConnect}, testInfo) => {
  await new WebScenarioRunner({page, testInfo, tonConnect}).run(localnetActivityScenario)
})
