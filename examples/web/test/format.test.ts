import {describe, expect, test} from "bun:test"

import {
  compactAddress,
  formatActivityAmount,
  formatBalance,
  formatUsdBalance,
  gramsToNanograms,
} from "@/lib/format"

describe("wallet display helpers", () => {
  test("keeps short addresses and compacts long addresses", () => {
    expect(compactAddress("0:short")).toBe("0:short")
    expect(compactAddress("EQAz8sBz-Twy965gFWNHlwa2ArkRLaoVzAowtRaW542bDO5p")).toBe(
      "EQAz8sBz…542bDO5p",
    )
  })

  test("limits balance precision without converting through a number", () => {
    expect(formatBalance("124.567890000")).toBe("124.5678")
    expect(formatBalance("8.000000000")).toBe("8")
    expect(formatBalance(undefined)).toBe("—")
  })

  test("uses the activity direction for the amount sign", () => {
    expect(formatActivityAmount("2.75", "received")).toBe("+2.75 GRAM")
    expect(formatActivityAmount("2.75", "sent")).toBe("−2.75 GRAM")
  })

  test("formats the current balance in US dollars", () => {
    expect(formatUsdBalance("2", 1.25)).toBe("$2.50")
    expect(formatUsdBalance(undefined, 1.25)).toBe("$—")
    expect(formatUsdBalance("2", undefined)).toBe("$—")
  })

  test("converts GRAM to nanograms without floating point", () => {
    expect(gramsToNanograms("1")).toBe("1000000000")
    expect(gramsToNanograms("12.000000001")).toBe("12000000001")
    expect(() => gramsToNanograms("0")).toThrow("greater than zero")
    expect(() => gramsToNanograms("1.0000000001")).toThrow("9 decimal places")
  })
})
