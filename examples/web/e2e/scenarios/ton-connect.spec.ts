import {WebScenarioRunner} from "../dsl/web-runner"
import {test} from "../fixtures/ton-connect"
import {
  rejectedTonConnectScenario,
  tenMessageTonConnectScenario,
  tonConnectScenario,
} from "./definitions"

test("connects and rejects a two-message transaction", async ({page, tonConnect}, testInfo) => {
  await new WebScenarioRunner({page, testInfo, tonConnect}).run(tonConnectScenario)
})

test("connects and rejects a ten-message transaction", async ({page, tonConnect}, testInfo) => {
  await new WebScenarioRunner({page, testInfo, tonConnect}).run(tenMessageTonConnectScenario)
})

test("rejects a connection and remains disconnected", async ({page, tonConnect}, testInfo) => {
  await new WebScenarioRunner({page, testInfo, tonConnect}).run(rejectedTonConnectScenario)
})
