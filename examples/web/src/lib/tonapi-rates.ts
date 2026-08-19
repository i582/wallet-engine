const TONAPI_RATE_URL: string =
  import.meta.env.VITE_TONAPI_RATE_URL ?? "https://tonapi.io/v2/rates?tokens=ton&currencies=usd"

export async function fetchGramUsdRate(
  fetcher: typeof globalThis.fetch = globalThis.fetch,
): Promise<number> {
  const response: Response = await fetcher(TONAPI_RATE_URL, {
    headers: {accept: "application/json"},
    redirect: "error",
  })
  if (!response.ok) {
    throw new Error(`TonAPI rates request failed with HTTP ${response.status}`)
  }
  return parseGramUsdRate(await response.json())
}

export function parseGramUsdRate(payload: unknown): number {
  const rates: unknown = readProperty(payload, "rates")
  const ton: unknown = readProperty(rates, "TON")
  const prices: unknown = readProperty(ton, "prices")
  const usd: unknown = readProperty(prices, "USD")
  if (typeof usd !== "number" || !Number.isFinite(usd) || usd <= 0) {
    throw new Error("TonAPI returned an invalid GRAM/USD rate")
  }
  return usd
}

function readProperty(value: unknown, property: string): unknown {
  if (typeof value !== "object" || value === null || !(property in value)) {
    throw new Error("TonAPI returned an invalid rates response")
  }
  return value[property as keyof typeof value]
}
