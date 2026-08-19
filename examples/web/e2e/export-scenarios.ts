import {mkdir, writeFile} from "node:fs/promises"
import path from "node:path"

import {clientScenarios} from "./scenarios/definitions"

const OUTPUT_DIRECTORY: string = path.resolve(import.meta.dirname, "../../client-e2e/scenarios")

/** Writes every serializable client scenario for non-TypeScript platform runners. */
async function exportScenarios(): Promise<void> {
  await mkdir(OUTPUT_DIRECTORY, {recursive: true})
  await Promise.all(
    Object.entries(clientScenarios).map(async ([name, definition]) => {
      const destination: string = path.join(OUTPUT_DIRECTORY, `${name}.json`)
      await writeFile(destination, `${JSON.stringify(definition, null, 2)}\n`, "utf8")
    }),
  )
}

await exportScenarios()
