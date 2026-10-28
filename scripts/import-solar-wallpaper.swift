// macOS import utility only; never part of the OxyOS runtime or image build.
// swift scripts/import-solar-wallpaper.swift source.heic destination-directory
import Foundation
import ImageIO
import CryptoKit

if CommandLine.arguments.count != 3 {
  fputs("Usage: swift scripts/import-solar-wallpaper.swift source.heic destination-directory\n", stderr)
  exit(1)
}
let sourceURL = URL(fileURLWithPath: CommandLine.arguments[1])
let destination = URL(fileURLWithPath: CommandLine.arguments[2], isDirectory: true)
guard let source = CGImageSourceCreateWithURL(sourceURL as CFURL, nil) else { fatalError("Cannot read HEIC") }
let framesURL = destination.appendingPathComponent("frames", isDirectory: true)
try FileManager.default.createDirectory(at: framesURL, withIntermediateDirectories: true)
let count = CGImageSourceGetCount(source)
var frames: [[String: Any]] = []
for index in 0..<count {
  let name = String(format: "frame-%02d.jpg", index)
  let output = framesURL.appendingPathComponent(name)
  guard let image = CGImageSourceCreateImageAtIndex(source, index, nil),
        let writer = CGImageDestinationCreateWithURL(output as CFURL, "public.jpeg" as CFString, 1, nil)
  else { fatalError("Cannot decode frame \(index)") }
  CGImageDestinationAddImage(writer, image, [kCGImageDestinationLossyCompressionQuality: 0.96] as CFDictionary)
  guard CGImageDestinationFinalize(writer) else { fatalError("Cannot save frame \(index)") }
  frames.append(["index": index, "file": "frames/\(name)", "width": image.width, "height": image.height])
  print("Saved", name)
}
guard let metadata = CGImageSourceCopyMetadataAtIndex(source, 0, nil),
      let tag = CGImageMetadataCopyStringValueWithPath(metadata, nil, "apple_desktop:solar" as CFString),
      let binary = Data(base64Encoded: tag as String),
      let original = try PropertyListSerialization.propertyList(from: binary, format: nil) as? [String: Any],
      let anchors = original["si"] as? [[String: Any]] else { fatalError("Missing solar metadata") }
let sourceData = try Data(contentsOf: sourceURL)
let manifest: [String: Any] = [
  "id": "solar-gradients", "name": "Solar Gradients", "version": 1,
  "source": ["filename": sourceURL.lastPathComponent, "sha256": SHA256.hash(data: sourceData).map { String(format: "%02x", $0) }.joined(), "bytes": sourceData.count, "frameCount": count, "colorProfile": "Display P3"],
  "encoding": "JPEG, quality 0.96, original resolution, embedded source colour profile",
  "previewFrame": 8,
  "solarAnchors": anchors.map { ["frame": $0["i"]!, "elevationDegrees": $0["a"]!, "azimuthDegrees": $0["z"]!] },
  "frames": frames
]
let options: JSONSerialization.WritingOptions = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
try JSONSerialization.data(withJSONObject: manifest, options: options).write(to: destination.appendingPathComponent("manifest.json"))
try JSONSerialization.data(withJSONObject: original, options: options).write(to: destination.appendingPathComponent("solar-original.json"))
try binary.write(to: destination.appendingPathComponent("solar-original.plist"))
print("Imported", count, "frames and original solar anchors.")
