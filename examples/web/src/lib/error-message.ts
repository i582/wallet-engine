/** Returns a user-facing message from JavaScript and WASM error values. */
export function errorMessage(cause: unknown): string {
  if (cause instanceof Error) {
    return cause.message
  }
  if (typeof cause === "string" && cause.trim().length > 0) {
    return cause
  }
  if (typeof cause === "object" && cause !== null) {
    const record: Record<string, unknown> = cause as Record<string, unknown>
    for (const property of ["message", "diagnostic"] as const) {
      const value: unknown = record[property]
      if (typeof value === "string" && value.trim().length > 0) {
        return value
      }
    }
  }
  return "The wallet operation failed"
}
