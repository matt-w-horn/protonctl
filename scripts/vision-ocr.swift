// Prints the text Apple Vision reads in each image named on the command
// line, one line of text per observation, top to bottom, with a line
// "=== <path>" before each image. Proof of concept for RFC Q23.
import Foundation
import Vision

for path in CommandLine.arguments.dropFirst() {
    print("=== \(path)")
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = true
    request.automaticallyDetectsLanguage = true
    let handler = VNImageRequestHandler(url: URL(fileURLWithPath: path))
    do {
        try handler.perform([request])
    } catch {
        FileHandle.standardError.write("\(path): \(error)\n".data(using: .utf8)!)
        continue
    }
    let lines = (request.results ?? [])
        .sorted { $0.boundingBox.midY > $1.boundingBox.midY }
        .compactMap { $0.topCandidates(1).first?.string }
    print(lines.joined(separator: "\n"))
}
