import UIKit
import UniformTypeIdentifiers
import XCTest

struct SnapshotCapture {
    let screenshot: XCUIScreenshot
    let masks: [CGRect]
}

enum SnapshotVerifierError: LocalizedError {
    case invalidImage(String)
    case missingBaseline(String)
    case sizeMismatch(expected: CGSize, actual: CGSize)
    case visualDifference(ratio: Double)

    var errorDescription: String? {
        switch self {
        case .invalidImage(let name):
            "Could not decode the \(name) snapshot image"
        case .missingBaseline(let path):
            "Snapshot baseline is missing at \(path). Run with UPDATE_IOS_SNAPSHOTS=1."
        case .sizeMismatch(let expected, let actual):
            "Snapshot size changed from \(expected) to \(actual)"
        case .visualDifference(let ratio):
            "Snapshot differs in \(String(format: "%.3f", ratio * 100)) percent of pixels"
        }
    }
}

@MainActor
final class SnapshotVerifier {
    private static let maximumDifferentPixelRatio = 0.001
    private static let channelTolerance: UInt8 = 3

    private let bundle: Bundle
    private let environment: [String: String]
    private unowned let testCase: XCTestCase

    /// Creates a verifier that reads bundled baselines and attaches masked captures.
    init(
        testCase: XCTestCase,
        bundle: Bundle,
        environment: [String: String] = ProcessInfo.processInfo.environment
    ) {
        self.testCase = testCase
        self.bundle = bundle
        var resolvedEnvironment = environment
        for name in [
            "IOS_SNAPSHOT_SOURCE_DIR",
            "IOS_SNAPSHOT_VARIANT",
            "UPDATE_IOS_SNAPSHOTS",
        ] {
            let currentValue = resolvedEnvironment[name]
            guard currentValue == nil || currentValue?.isEmpty == true else {
                continue
            }
            guard let bundledValue = bundle.object(forInfoDictionaryKey: name) as? String,
                  !bundledValue.isEmpty,
                  !bundledValue.hasPrefix("$(") else {
                continue
            }
            resolvedEnvironment[name] = bundledValue
        }
        self.environment = resolvedEnvironment
    }

    /// Updates or compares one named screenshot for the configured simulator variant.
    func verify(name: String, capture: SnapshotCapture) throws {
        let actual = try maskedPNGData(capture, name: name)
        attach(actual, name: "\(name)-actual")

        if environment["UPDATE_IOS_SNAPSHOTS"] == "1" {
            try writeBaseline(actual, name: name)
            return
        }

        guard let baselineURL = baselineURL(name: name) else {
            throw SnapshotVerifierError.missingBaseline("Snapshots/\(variant)/\(name).png")
        }
        let expected = try Data(contentsOf: baselineURL)
        do {
            try compare(expected: expected, actual: actual)
        } catch {
            attach(expected, name: "\(name)-expected")
            throw error
        }
    }

    /// Returns the stable directory name selected by the iOS E2E harness.
    private var variant: String {
        environment["IOS_SNAPSHOT_VARIANT"] ?? "iphone-16-pro"
    }

    /// Finds a source-tree baseline from the harness or its bundled fallback.
    private func baselineURL(name: String) -> URL? {
        if let sourceDirectory = environment["IOS_SNAPSHOT_SOURCE_DIR"] {
            let sourceURL = URL(fileURLWithPath: sourceDirectory, isDirectory: true)
                .appendingPathComponent(variant, isDirectory: true)
                .appendingPathComponent("\(name).png")
            if FileManager.default.fileExists(atPath: sourceURL.path) {
                return sourceURL
            }
        }
        return bundle.url(
            forResource: name,
            withExtension: "png",
            subdirectory: "Snapshots/\(variant)"
        )
    }

    /// Applies opaque masks before secret-bearing screenshots can leave test memory.
    private func maskedPNGData(
        _ capture: SnapshotCapture,
        name: String
    ) throws -> Data {
        let sourceData = capture.screenshot.pngRepresentation
        guard let source = UIImage(data: sourceData),
              let sourceImage = source.cgImage else {
            throw SnapshotVerifierError.invalidImage(name)
        }
        let pixelSize = CGSize(width: sourceImage.width, height: sourceImage.height)
        let pointSize = capture.screenshot.image.size
        let scaleX = pointSize.width == 0 ? 1 : pixelSize.width / pointSize.width
        let scaleY = pointSize.height == 0 ? 1 : pixelSize.height / pointSize.height
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = true
        let renderer = UIGraphicsImageRenderer(size: pixelSize, format: format)
        let image = renderer.image { context in
            context.cgContext.interpolationQuality = .none
            UIImage(cgImage: sourceImage).draw(in: CGRect(origin: .zero, size: pixelSize))
            UIColor.magenta.setFill()
            for mask in capture.masks {
                let scaled = CGRect(
                    x: mask.minX * scaleX,
                    y: mask.minY * scaleY,
                    width: mask.width * scaleX,
                    height: mask.height * scaleY
                ).insetBy(dx: -2, dy: -2)
                context.cgContext.fill(scaled)
            }
        }
        guard let data = image.pngData() else {
            throw SnapshotVerifierError.invalidImage(name)
        }
        return data
    }

    /// Writes an updated baseline into the repository path supplied by the harness.
    private func writeBaseline(_ data: Data, name: String) throws {
        guard let sourceDirectory = environment["IOS_SNAPSHOT_SOURCE_DIR"] else {
            throw SnapshotVerifierError.missingBaseline("IOS_SNAPSHOT_SOURCE_DIR")
        }
        let directory = URL(fileURLWithPath: sourceDirectory, isDirectory: true)
            .appendingPathComponent(variant, isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true
        )
        try data.write(to: directory.appendingPathComponent("\(name).png"), options: .atomic)
    }

    /// Compares decoded RGBA pixels with a small anti-aliasing tolerance.
    private func compare(expected: Data, actual: Data) throws {
        guard let expectedImage = UIImage(data: expected)?.cgImage else {
            throw SnapshotVerifierError.invalidImage("expected")
        }
        guard let actualImage = UIImage(data: actual)?.cgImage else {
            throw SnapshotVerifierError.invalidImage("actual")
        }
        let expectedSize = CGSize(width: expectedImage.width, height: expectedImage.height)
        let actualSize = CGSize(width: actualImage.width, height: actualImage.height)
        guard expectedSize == actualSize else {
            throw SnapshotVerifierError.sizeMismatch(expected: expectedSize, actual: actualSize)
        }
        let expectedPixels = try rgbaPixels(expectedImage)
        let actualPixels = try rgbaPixels(actualImage)
        var differentPixels = 0
        for pixel in stride(from: 0, to: expectedPixels.count, by: 4) {
            let differs = (0..<4).contains { channel in
                abs(
                    Int(expectedPixels[pixel + channel])
                        - Int(actualPixels[pixel + channel])
                ) > Int(Self.channelTolerance)
            }
            if differs {
                differentPixels += 1
            }
        }
        let pixelCount = expectedPixels.count / 4
        let ratio = pixelCount == 0 ? 0 : Double(differentPixels) / Double(pixelCount)
        guard ratio <= Self.maximumDifferentPixelRatio else {
            throw SnapshotVerifierError.visualDifference(ratio: ratio)
        }
    }

    /// Renders one image into deterministic eight-bit RGBA bytes.
    private func rgbaPixels(_ image: CGImage) throws -> [UInt8] {
        let bytesPerRow = image.width * 4
        var pixels = [UInt8](repeating: 0, count: bytesPerRow * image.height)
        guard let context = CGContext(
            data: &pixels,
            width: image.width,
            height: image.height,
            bitsPerComponent: 8,
            bytesPerRow: bytesPerRow,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else {
            throw SnapshotVerifierError.invalidImage("RGBA")
        }
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        return pixels
    }

    /// Attaches one masked PNG to the XCTest result bundle.
    private func attach(_ data: Data, name: String) {
        let attachment = XCTAttachment(
            data: data,
            uniformTypeIdentifier: UTType.png.identifier
        )
        attachment.name = name
        attachment.lifetime = .keepAlways
        testCase.add(attachment)
    }
}
