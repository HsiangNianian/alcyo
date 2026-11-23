// alcyo-speech: Apple's speech recognition for Alcyo, one sentence at a time.
//
//   alcyo-speech serve <locale|auto>
//     Prints {"type":"ready","engine":"analyzer"|"legacy","locale":…} once it can recognize, or
//     {"type":"error","message":…} and exits. Then, for each sentence, reads a little-endian UInt32
//     sample count and that many 16 kHz mono float32 samples from stdin, and prints
//     {"type":"text","text":…} or {"type":"error","message":…}. A count of 0, or the end of stdin, ends it.
//   alcyo-speech self-test
//     Checks the audio conversion, for the build.
//
// SpeechAnalyzer (macOS 26, iPadOS 26) is used when it supports the language and its model can be installed;
// otherwise SFSpeechRecognizer, on the device when it can be.
//
// On the Mac this is a program of its own. iPadOS lets an app start no other program, so there it is built with
// -D ALCYO_SPEECH_LIBRARY into the app instead, and alcyo_speech_serve speaks the same protocol over two pipes
// the app holds the other ends of.

import AVFoundation
import Foundation
import Speech

/// Where sentences come from and answers go: stdin and stdout for the program, pipes in the app.
final class Channel {
    let input: FileHandle
    let output: FileHandle
    private let lock = NSLock()

    init(input: FileHandle, output: FileHandle) {
        self.input = input
        self.output = output
    }

    func emit(_ event: [String: Any]) {
        guard var data = try? JSONSerialization.data(withJSONObject: event, options: [.sortedKeys]) else { return }
        data.append(10)
        lock.lock()
        defer { lock.unlock() }
        output.write(data)
    }

    /// Exactly `count` bytes, or nil at the input's end.
    func readExactly(_ count: Int) -> Data? {
        var data = Data()
        while data.count < count {
            guard let chunk = try? input.read(upToCount: count - data.count), !chunk.isEmpty else { return nil }
            data.append(chunk)
        }
        return data
    }

    /// The next sentence's samples, or nil when there are no more.
    func nextSentence() -> [Float]? {
        guard let header = readExactly(4) else { return nil }
        let count = Int(header.withUnsafeBytes { UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self)) })
        if count == 0 { return nil }
        guard let body = readExactly(count * 4) else { return nil }
        return body.withUnsafeBytes { raw in
            (0..<count).map { Float(bitPattern: UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: $0 * 4, as: UInt32.self))) }
        }
    }
}

struct Failure: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

let inputFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false)!

/// The samples as a buffer in the format they arrive in.
func buffer(_ samples: [Float]) -> AVAudioPCMBuffer {
    let buffer = AVAudioPCMBuffer(pcmFormat: inputFormat, frameCapacity: AVAudioFrameCount(max(samples.count, 1)))!
    buffer.frameLength = AVAudioFrameCount(samples.count)
    samples.withUnsafeBufferPointer { source in
        if let base = source.baseAddress {
            buffer.floatChannelData![0].update(from: base, count: samples.count)
        }
    }
    return buffer
}

/// The samples in `format`, which the recognizer asks for.
func convert(_ samples: [Float], to format: AVAudioFormat) throws -> AVAudioPCMBuffer {
    let input = buffer(samples)
    if format == inputFormat { return input }
    guard let converter = AVAudioConverter(from: inputFormat, to: format) else {
        throw Failure(message: "Cannot convert audio to Apple's input format")
    }
    converter.primeMethod = .none
    let capacity = AVAudioFrameCount(ceil(Double(samples.count) * format.sampleRate / 16_000)) + 64
    let output = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity)!
    var supplied = false
    var error: NSError?
    let status = converter.convert(to: output, error: &error) { _, state in
        if supplied {
            state.pointee = .endOfStream
            return nil
        }
        supplied = true
        state.pointee = .haveData
        return input
    }
    if let error { throw error }
    if status == .error { throw Failure(message: "Audio conversion failed") }
    return output
}

protocol Engine {
    var name: String { get }
    func recognize(_ samples: [Float]) async throws -> String
}

@available(macOS 26.0, iOS 26.0, *)
final class Analyzer: Engine {
    let name = "analyzer"
    let locale: Locale

    init(locale: Locale) {
        self.locale = locale
    }

    func module() -> SpeechTranscriber {
        SpeechTranscriber(locale: locale, transcriptionOptions: [], reportingOptions: [], attributeOptions: [])
    }

    /// The language model is on the device, downloading it if the system allows.
    static func start(_ wanted: Locale, _ channel: Channel) async throws -> Analyzer {
        guard SpeechTranscriber.isAvailable else { throw Failure(message: "SpeechAnalyzer is not available on this device") }
        guard let locale = await SpeechTranscriber.supportedLocale(equivalentTo: wanted) else {
            throw Failure(message: "SpeechAnalyzer does not support \(wanted.identifier)")
        }
        let analyzer = Analyzer(locale: locale)
        let module = analyzer.module()
        if await AssetInventory.status(forModules: [module]) != .installed {
            channel.emit(["type": "preparing", "locale": locale.identifier])
            if let request = try await AssetInventory.assetInstallationRequest(supporting: [module]) {
                try await request.downloadAndInstall()
            }
        }
        guard await AssetInventory.status(forModules: [module]) == .installed else {
            throw Failure(message: "The speech model for \(locale.identifier) is not installed")
        }
        return analyzer
    }

    func recognize(_ samples: [Float]) async throws -> String {
        let module = module()
        guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [module]) else {
            throw Failure(message: "Apple could not provide a compatible audio format")
        }
        let audio = try convert(samples, to: format)
        let analyzer = SpeechAnalyzer(modules: [module])
        try await analyzer.prepareToAnalyze(in: format)
        let results = Task { () throws -> String in
            var text = ""
            for try await result in module.results where result.isFinal {
                text += String(result.text.characters)
            }
            return text
        }
        let (input, feed) = AsyncStream<AnalyzerInput>.makeStream()
        feed.yield(AnalyzerInput(buffer: audio))
        feed.finish()
        do {
            if let last = try await analyzer.analyzeSequence(input) {
                try await analyzer.finalizeAndFinish(through: last)
            } else {
                await analyzer.cancelAndFinishNow()
            }
            return try await results.value.trimmingCharacters(in: .whitespacesAndNewlines)
        } catch {
            await analyzer.cancelAndFinishNow()
            results.cancel()
            throw error
        }
    }
}

/// Only the first call to `finish` counts: the recognizer can report again after its final result.
final class Once<T> {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<T, Error>?

    init(_ continuation: CheckedContinuation<T, Error>) {
        self.continuation = continuation
    }

    func finish(_ result: Result<T, Error>) {
        lock.lock()
        let waiting = continuation
        continuation = nil
        lock.unlock()
        waiting?.resume(with: result)
    }
}

final class Legacy: Engine {
    let name = "legacy"
    let recognizer: SFSpeechRecognizer

    init(recognizer: SFSpeechRecognizer) {
        self.recognizer = recognizer
    }

    static func start(_ wanted: Locale) async throws -> Legacy {
        let status = await withCheckedContinuation { done in
            SFSpeechRecognizer.requestAuthorization { done.resume(returning: $0) }
        }
        guard status == .authorized else {
            #if os(iOS)
            throw Failure(message: "Speech recognition is not allowed. Turn it on for Alcyo in Settings › Privacy & Security › Speech Recognition.")
            #else
            throw Failure(message: "Speech recognition is not allowed. Turn it on for Alcyo in System Settings › Privacy & Security › Speech Recognition.")
            #endif
        }
        // The system's default recognizer stands in only when it hears the same language, such as en_US for an
        // English Mac set to a region Apple has no recognizer for; never one for another language.
        let fallback = SFSpeechRecognizer().flatMap { $0.locale.language.languageCode == wanted.language.languageCode ? $0 : nil }
        guard let recognizer = SFSpeechRecognizer(locale: wanted) ?? fallback, recognizer.isAvailable else {
            throw Failure(message: "Apple speech recognition is not available for \(wanted.identifier)")
        }
        // Without this the recognizer sends the audio to Apple's servers, and meetings stay on this Mac.
        guard recognizer.supportsOnDeviceRecognition else {
            throw Failure(message: "This device can't recognize \(recognizer.locale.identifier) on its own, and Apple would send the meeting's audio to its servers")
        }
        // Its answers come on this queue rather than the main one, which nothing here services.
        recognizer.queue = OperationQueue()
        return Legacy(recognizer: recognizer)
    }

    func recognize(_ samples: [Float]) async throws -> String {
        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = false
        // Checked when starting; required here too, so a recognizer that changes its mind fails rather than uploads.
        request.requiresOnDeviceRecognition = true
        if #available(macOS 13.0, iOS 16.0, *) {
            request.addsPunctuation = true
        }
        request.append(buffer(samples))
        request.endAudio()
        var task: SFSpeechRecognitionTask?
        defer { task?.cancel() }
        return try await withCheckedThrowingContinuation { continuation in
            let once = Once(continuation)
            task = recognizer.recognitionTask(with: request) { result, error in
                if let result, result.isFinal {
                    once.finish(.success(result.bestTranscription.formattedString))
                } else if let error = error as NSError? {
                    // "No speech detected" is an empty sentence, not a failure.
                    once.finish(error.code == 1110 || error.code == 203 ? .success("") : .failure(error))
                }
            }
        }
    }
}

func engine(for wanted: Locale, _ channel: Channel) async throws -> Engine {
    if #available(macOS 26.0, iOS 26.0, *) {
        do {
            return try await Analyzer.start(wanted, channel)
        } catch {
            channel.emit(["type": "fallback", "message": error.localizedDescription])
        }
    }
    return try await Legacy.start(wanted)
}

func serve(_ identifier: String, _ channel: Channel) async throws {
    let wanted = identifier == "auto" ? Locale.current : Locale(identifier: identifier)
    let engine = try await engine(for: wanted, channel)
    channel.emit(["type": "ready", "engine": engine.name, "locale": wanted.identifier])
    while let samples = channel.nextSentence() {
        do {
            channel.emit(["type": "text", "text": try await engine.recognize(samples)])
        } catch {
            channel.emit(["type": "error", "message": error.localizedDescription])
        }
    }
}

func selfTest(_ channel: Channel) throws {
    let values: [Float] = [0.25, -0.5, 0.75, 0]
    let same = try convert(values, to: inputFormat)
    guard Array(UnsafeBufferPointer(start: same.floatChannelData![0], count: Int(same.frameLength))) == values else {
        throw Failure(message: "Audio buffer self-test failed")
    }
    let doubled = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: 32_000, channels: 1, interleaved: false)!
    let resampled = try convert([Float](repeating: 0.1, count: 1600), to: doubled)
    guard resampled.frameLength > 2_800 && resampled.frameLength <= 3_264 else {
        throw Failure(message: "Resampling self-test failed: \(resampled.frameLength) frames")
    }
    channel.emit(["type": "done"])
}

#if ALCYO_SPEECH_LIBRARY
/// The app's way in: recognize for `locale` ("auto" for the system's language), reading sentences from the
/// file descriptor `input` and writing answers to `output`, as the program does with stdin and stdout. Returns
/// once the input ends or says so with a count of 0, and closes both. Blocks its thread meanwhile.
@_cdecl("alcyo_speech_serve")
public func alcyoSpeechServe(_ input: Int32, _ output: Int32, _ locale: UnsafePointer<CChar>) {
    let channel = Channel(input: FileHandle(fileDescriptor: input, closeOnDealloc: true), output: FileHandle(fileDescriptor: output, closeOnDealloc: true))
    let identifier = String(cString: locale)
    let finished = DispatchSemaphore(value: 0)
    Task.detached {
        do {
            try await serve(identifier, channel)
        } catch {
            channel.emit(["type": "error", "message": error.localizedDescription])
        }
        finished.signal()
    }
    finished.wait()
}
#else
@main
struct Main {
    static func main() async {
        let channel = Channel(input: FileHandle.standardInput, output: FileHandle.standardOutput)
        // Leave if Alcyo goes away without closing stdin.
        let parent = getppid()
        let watchdog = DispatchSource.makeTimerSource(queue: .global())
        watchdog.schedule(deadline: .now() + 1, repeating: 1)
        watchdog.setEventHandler { if getppid() != parent { exit(0) } }
        watchdog.resume()
        do {
            let args = Array(CommandLine.arguments.dropFirst())
            switch args.first {
            case "self-test": try selfTest(channel)
            case "serve" where args.count == 2: try await serve(args[1], channel)
            default: throw Failure(message: "Usage: alcyo-speech serve <locale|auto> | self-test")
            }
        } catch {
            channel.emit(["type": "error", "message": error.localizedDescription])
            exit(1)
        }
    }
}
#endif
