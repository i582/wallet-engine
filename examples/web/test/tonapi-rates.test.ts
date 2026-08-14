import {describe, expect, test} from "bun:test"

import {parseGramUsdRate} from "@/lib/tonapi-rates"

describe("TonAPI rates", () => {
  test("reads the GRAM price in US dollars", () => {
    expect(
      parseGramUsdRate({
        rates: {
          TON: {
            prices: {USD: 1.25},
          },
        },
      }),
    ).toBe(1.25)
  })

  test("rejects missing and invalid prices", () => {
    expect(() => parseGramUsdRate({rates: {}})).toThrow("invalid rates response")
    expect(() => parseGramUsdRate({rates: {TON: {prices: {USD: Number.NaN}}}})).toThrow(
      "invalid GRAM/USD rate",
    )
  })
})
