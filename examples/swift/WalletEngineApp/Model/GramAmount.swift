import Foundation

/// Exact decimal conversion for the  send boundary.
///
/// GRAM has nine fractional decimal places. This parser intentionally never
/// passes through `Double` or `Decimal`, so the unsigned nanogram amount is the
/// exact value the user entered.
nonisolated enum GramAmount {
    static func nanograms(from input: String) -> String? {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed.utf8.count <= 128 else { return nil }

        let separator: Character?
        if trimmed.contains(".") && trimmed.contains(",") {
            return nil
        } else if trimmed.contains(".") {
            separator = "."
        } else if trimmed.contains(",") {
            separator = ","
        } else {
            separator = nil
        }

        let components: [Substring]
        if let separator {
            components = trimmed.split(
                separator: separator,
                maxSplits: 1,
                omittingEmptySubsequences: false
            )
        } else {
            components = [Substring(trimmed)]
        }
        guard components.count <= 2 else { return nil }

        let whole = components[0]
        let fraction = components.count == 2 ? components[1] : Substring()
        guard !whole.isEmpty,
              whole.utf8.allSatisfy({ (48...57).contains($0) }),
              fraction.utf8.allSatisfy({ (48...57).contains($0) }),
              fraction.count <= 9 else {
            return nil
        }

        let normalizedWhole = whole.drop(while: { $0 == "0" })
        let wholeDigits = normalizedWhole.isEmpty ? "0" : String(normalizedWhole)
        let fractionalDigits = String(fraction) + String(repeating: "0", count: 9 - fraction.count)
        let combined = wholeDigits + fractionalDigits
        let canonical = combined.drop(while: { $0 == "0" })
        return canonical.isEmpty ? "0" : String(canonical)
    }

    static func format(nanograms: String) -> String {
        let digits = nanograms.drop(while: { $0 == "0" })
        let canonical = digits.isEmpty ? "0" : String(digits)
        guard canonical.utf8.allSatisfy({ (48...57).contains($0) }) else {
            return "—"
        }

        if canonical.count <= 9 {
            let fraction = String(repeating: "0", count: 9 - canonical.count) + canonical
            let trimmed = fraction.reversed()
                .drop(while: { $0 == "0" }).reversed()
            return trimmed.isEmpty ? "0" : "0.\(String(trimmed))"
        }

        let splitIndex = canonical.index(canonical.endIndex, offsetBy: -9)
        let whole = canonical[..<splitIndex]
        let fraction = canonical[splitIndex...].reversed()
            .drop(while: { $0 == "0" }).reversed()
        return fraction.isEmpty ? String(whole) : "\(whole).\(String(fraction))"
    }
}
