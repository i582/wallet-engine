import type {ActivityItem} from "@ton/wallet-engine"

type ActivityDirection = ActivityItem["direction"]

const TRAILING_ZEROES: RegExp = /0+$/
const GRAM_AMOUNT: RegExp = /^(?:0|[1-9]\d*)(?:\.\d{1,9})?$/

export function compactAddress(address: string): string {
  if (address.length <= 18) {
    return address
  }
  return `${address.slice(0, 8)}…${address.slice(-8)}`
}

export function formatBalance(value: string | undefined): string {
  if (!value) {
    return "—"
  }
  const [integer, fraction = ""]: string[] = value.split(".")
  const trimmedFraction: string = fraction.slice(0, 4).replace(TRAILING_ZEROES, "")
  return trimmedFraction ? `${integer}.${trimmedFraction}` : integer
}

export function formatUsdBalance(
  balanceGrams: string | undefined,
  gramUsdRate: number | undefined,
): string {
  if (!balanceGrams || gramUsdRate === undefined) {
    return "$—"
  }
  const value: number = Number(balanceGrams) * gramUsdRate
  if (!Number.isFinite(value)) {
    return "$—"
  }
  return new Intl.NumberFormat(undefined, {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(value)
}

export function formatActivityAmount(amount: string, direction: ActivityDirection): string {
  const sign: string = direction === "received" ? "+" : "−"
  return `${sign}${formatBalance(amount)} GRAM`
}

export function formatTimestamp(timestamp: number): string {
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(timestamp * 1000))
}

export function gramsToNanograms(value: string): string {
  const normalized: string = value.trim()
  if (!GRAM_AMOUNT.test(normalized)) {
    throw new Error("Enter a positive amount with no more than 9 decimal places")
  }
  const [grams, fraction = ""]: string[] = normalized.split(".")
  const nanograms: bigint = BigInt(grams) * 1_000_000_000n + BigInt(fraction.padEnd(9, "0"))
  if (nanograms === 0n) {
    throw new Error("The amount must be greater than zero")
  }
  return nanograms.toString()
}
