import type {SseEvent} from "./ton-connect-types"

const TRAILING_CR: RegExp = /\r$/u

export class SseParser {
  private readonly decoder: TextDecoder = new TextDecoder()
  private buffer: string = ""
  private eventType: string = "message"
  private dataLines: string[] = []
  private eventId?: string

  push(chunk: Uint8Array): SseEvent[] {
    this.buffer += this.decoder.decode(chunk, {stream: true})
    const events: SseEvent[] = []
    while (true) {
      const newline: number = this.buffer.indexOf("\n")
      if (newline < 0) {
        break
      }
      const line: string = this.buffer.slice(0, newline).replace(TRAILING_CR, "")
      this.buffer = this.buffer.slice(newline + 1)
      const event: SseEvent | undefined = this.processLine(line)
      if (event !== undefined) {
        events.push(event)
      }
    }
    return events
  }

  private processLine(line: string): SseEvent | undefined {
    if (line.length === 0) {
      const event: SseEvent | undefined =
        this.dataLines.length > 0 || this.eventType === "heartbeat"
          ? {id: this.eventId, event: this.eventType, data: this.dataLines.join("\n")}
          : undefined
      this.eventType = "message"
      this.dataLines = []
      return event
    }
    if (line.startsWith(":")) {
      return undefined
    }
    const separator: number = line.indexOf(":")
    const field: string = separator < 0 ? line : line.slice(0, separator)
    const rawValue: string = separator < 0 ? "" : line.slice(separator + 1)
    const value: string = rawValue.startsWith(" ") ? rawValue.slice(1) : rawValue
    if (field === "event") {
      this.eventType = value
    } else if (field === "data") {
      this.dataLines.push(value)
    } else if (field === "id" && !value.includes("\0")) {
      this.eventId = value
    }
    return undefined
  }
}
