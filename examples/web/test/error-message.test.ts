import {describe, expect, test} from "bun:test"

import {errorMessage} from "@/lib/error-message"

describe("errorMessage", () => {
  test("preserves Error and non-empty string messages", () => {
    expect(errorMessage(new Error("failed"))).toBe("failed")
    expect(errorMessage("failed in host")).toBe("failed in host")
  })

  test("reads structured JavaScript and WASM error fields", () => {
    expect(errorMessage({message: "JavaScript failure"})).toBe("JavaScript failure")
    expect(errorMessage({diagnostic: "WASM failure", kind: "invalidInput"})).toBe("WASM failure")
  })

  test("uses a stable fallback for values without a message", () => {
    expect(errorMessage(null)).toBe("The wallet operation failed")
    expect(errorMessage({diagnostic: ""})).toBe("The wallet operation failed")
  })
})
