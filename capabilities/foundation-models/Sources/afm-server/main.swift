// Serves Apple's on-device model over the OpenAI chat-completions shape, so
// Axon reaches it as a backend in inference.json rather than through a binding.
//
// Why a shim and not FFI: `libs/summarize::complete` already speaks
// chat-completions, and every consumer resolves its endpoint from config. A
// shim makes this a config entry and leaves that library untouched. The
// alternative would have put a Swift bridge in the one Rust file every
// capability compiles in by `#[path]` include.
//
// No package dependencies. Network.framework ships in the SDK, and a
// twenty-line HTTP reader is cheaper than a server framework in a binary whose
// entire surface is two routes on loopback.

import Foundation
import FoundationModels
import Network

// MARK: - Wire types

/// The subset of the OpenAI request this serves. Unknown fields are ignored
/// rather than rejected: callers send `stream`, `top_p` and others that do not
/// apply here, and refusing them would make this a special case to configure
/// instead of a drop-in endpoint.
struct ChatRequest: Decodable {
    struct Message: Decodable {
        let role: String
        let content: String
    }
    let messages: [Message]
    let maxTokens: Int?

    enum CodingKeys: String, CodingKey {
        case messages
        case maxTokens = "max_tokens"
    }
}

/// The OpenAI error envelope. Deliberately the same shape oMLX uses when its
/// memory guard fires, because Axon reads this before it reads `choices` and
/// classifies it as a machine condition rather than an empty answer.
struct ErrorEnvelope: Encodable, Error {
    struct Detail: Encodable {
        let message: String
        let type: String
        let code: String
    }
    let error: Detail
    let type = "error"

    init(message: String, code: String) {
        self.error = Detail(message: message, type: "invalid_request_error", code: code)
    }
}

// MARK: - The model

/// Apple's on-device model has a fixed window shared by prompt and answer.
/// Read from the framework rather than hardcoded, so an OS update that changes
/// it changes this too.
let contextSize = SystemLanguageModel.default.contextSize

/// Refuse rather than truncate.
///
/// A silently shortened prompt produces a digest of the first half of a
/// document and says nothing about it, which is worse than no digest: the
/// reader cannot tell. An explicit error routes back through Axon's normal
/// failure path, where the ladder can pick the larger model instead.
///
/// Characters per token matches `libs/summarize::CHARS_PER_TOKEN`, and for the
/// same reason: measured at 3.94 on real transcripts, floored to 3 so the guard
/// errs toward refusing work it might actually have fit.
let charsPerToken = 3

func availabilityFailure() -> String? {
    switch SystemLanguageModel.default.availability {
    case .available:
        return nil
    case .unavailable(let reason):
        return "Apple's on-device model is unavailable on this host: \(reason)"
    @unknown default:
        return "Apple's on-device model reported an availability state this build does not know"
    }
}

func complete(_ request: ChatRequest) async -> Result<String, ErrorEnvelope> {
    // Roles are flattened rather than mapped onto Instructions: every caller
    // here sends a single user turn, and inventing a system/user split for a
    // one-shot digest would change the prompt the ladder was tuned against.
    let prompt = request.messages.map(\.content).joined(separator: "\n\n")
    let maxTokens = request.maxTokens ?? 512
    let estimated = prompt.count / charsPerToken + maxTokens

    guard estimated <= contextSize else {
        return .failure(ErrorEnvelope(
            message: """
                Prompt and reply together need about \(estimated) tokens, and this model's \
                context is \(contextSize). Send less source or use a model with a larger window.
                """,
            code: "context_length_exceeded"
        ))
    }

    do {
        let session = LanguageModelSession()
        let response = try await session.respond(
            to: prompt,
            options: GenerationOptions(maximumResponseTokens: maxTokens)
        )
        return .success(response.content)
    } catch let error as LanguageModelSession.GenerationError {
        // The framework's own window check, in case the estimate above was
        // generous. Same code, so a caller handles one case rather than two.
        if case .exceededContextWindowSize = error {
            return .failure(ErrorEnvelope(
                message: "the model refused the prompt as too large for its \(contextSize) token context",
                code: "context_length_exceeded"
            ))
        }
        return .failure(ErrorEnvelope(message: "\(error)", code: "generation_failed"))
    } catch {
        return .failure(ErrorEnvelope(message: "\(error)", code: "generation_failed"))
    }
}

// MARK: - Responses

func jsonBody(_ value: some Encodable) -> Data {
    (try? JSONEncoder().encode(value)) ?? Data(#"{"error":{"message":"could not encode response"}}"#.utf8)
}

func httpResponse(status: Int, reason: String, body: Data) -> Data {
    var head = "HTTP/1.1 \(status) \(reason)\r\n"
    head += "Content-Type: application/json\r\n"
    head += "Content-Length: \(body.count)\r\n"
    head += "Connection: close\r\n\r\n"
    return Data(head.utf8) + body
}

/// The completion shape Axon reads. `id`/`created` are present because clients
/// expect the envelope, not because anything here uses them.
func completionBody(_ text: String) -> Data {
    let payload: [String: Any] = [
        "id": "afm-\(UUID().uuidString)",
        "object": "chat.completion",
        "created": Int(Date().timeIntervalSince1970),
        "model": "apple-on-device",
        "choices": [[
            "index": 0,
            "message": ["role": "assistant", "content": text],
            "finish_reason": "stop",
        ]],
    ]
    return (try? JSONSerialization.data(withJSONObject: payload)) ?? Data()
}

// MARK: - A very small HTTP/1.1 reader

/// Reads one request off the connection and hands back method, path and body.
/// Keep-alive is not supported and the response always closes: this serves one
/// local consumer making one blocking call at a time.
func readRequest(_ connection: NWConnection, _ done: @escaping @Sendable (String, String, Data) -> Void) {
    // The buffer travels as a parameter rather than a captured var. NWConnection
    // serialises a connection's callbacks, so a shared var would in fact be
    // safe, but "in fact safe" is the kind of claim that stops being true when
    // someone adds a queue -- and Swift 6 is right to ask.
    @Sendable func pump(_ carried: Data) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 1 << 20) { chunk, _, isComplete, error in
            var buffer = carried
            if let chunk { buffer.append(chunk) }
            if error != nil { connection.cancel(); return }

            guard let headerEnd = buffer.range(of: Data("\r\n\r\n".utf8)) else {
                if isComplete { connection.cancel() } else { pump(buffer) }
                return
            }
            let head = String(decoding: buffer[..<headerEnd.lowerBound], as: UTF8.self)
            let lines = head.split(separator: "\r\n", omittingEmptySubsequences: false)
            let request = lines.first?.split(separator: " ") ?? []
            let method = request.count > 0 ? String(request[0]) : ""
            let path = request.count > 1 ? String(request[1]) : ""

            let contentLength = lines.dropFirst()
                .first { $0.lowercased().hasPrefix("content-length:") }
                .flatMap { Int($0.split(separator: ":")[1].trimmingCharacters(in: .whitespaces)) } ?? 0

            let body = buffer[headerEnd.upperBound...]
            // A body split across TCP segments is the normal case for a 15 KB
            // transcript, so this waits for all of it rather than parsing what
            // arrived first.
            if body.count < contentLength, !isComplete {
                pump(buffer)
                return
            }
            done(method, path, Data(body.prefix(contentLength)))
        }
    }
    pump(Data())
}

func send(_ connection: NWConnection, _ data: Data) {
    connection.send(content: data, completion: .contentProcessed { _ in connection.cancel() })
}

func handle(_ connection: NWConnection) {
    connection.start(queue: .global())
    readRequest(connection) { method, path, body in
        switch (method, path) {
        case ("GET", "/health"):
            let payload: [String: Any] = [
                "status": availabilityFailure() == nil ? "ok" : "unavailable",
                "model": "apple-on-device",
                "context_tokens": contextSize,
            ]
            let data = (try? JSONSerialization.data(withJSONObject: payload)) ?? Data()
            send(connection, httpResponse(status: 200, reason: "OK", body: data))

        case ("POST", "/v1/chat/completions"):
            guard let request = try? JSONDecoder().decode(ChatRequest.self, from: body) else {
                let envelope = ErrorEnvelope(message: "could not parse the request body", code: "invalid_request")
                send(connection, httpResponse(status: 400, reason: "Bad Request", body: jsonBody(envelope)))
                return
            }
            Task {
                switch await complete(request) {
                case .success(let text):
                    send(connection, httpResponse(status: 200, reason: "OK", body: completionBody(text)))
                case .failure(let envelope):
                    // 200, matching oMLX: the envelope is what carries the
                    // failure, and Axon reads it before `choices`. Returning a
                    // 4xx would be more correct in the abstract and less useful
                    // in practice, because the one consumer reads any non-2xx
                    // as "server down" rather than "this request will not fit".
                    send(connection, httpResponse(status: 200, reason: "OK", body: jsonBody(envelope)))
                }
            }

        default:
            let envelope = ErrorEnvelope(message: "no route for \(method) \(path)", code: "not_found")
            send(connection, httpResponse(status: 404, reason: "Not Found", body: jsonBody(envelope)))
        }
    }
}

// MARK: - Entry

let arguments = CommandLine.arguments

// `--check` answers "would this work here" without binding a port, which is
// what a host that may not be a Mac needs before enabling the capability.
if arguments.contains("--check") {
    if let failure = availabilityFailure() {
        FileHandle.standardError.write(Data("foundation-models: \(failure)\n".utf8))
        exit(1)
    }
    print("foundation-models: available, context \(contextSize) tokens")
    exit(0)
}

// Refuse to start rather than bind a port and fail every request. A host
// without Apple Intelligence, or without the framework at all, should look
// unhealthy immediately rather than accept work it cannot do.
if let failure = availabilityFailure() {
    FileHandle.standardError.write(Data("foundation-models: \(failure)\n".utf8))
    exit(1)
}

let port = ProcessInfo.processInfo.environment["AXON_PORT"].flatMap(UInt16.init) ?? 8091
guard let nwPort = NWEndpoint.Port(rawValue: port) else {
    FileHandle.standardError.write(Data("foundation-models: \(port) is not a usable port\n".utf8))
    exit(1)
}

// Loopback only, and not configurable. This endpoint has no authentication
// because it needs none on 127.0.0.1; the moment it listened on a routable
// address that would stop being true, so it cannot.
let parameters = NWParameters.tcp
parameters.requiredLocalEndpoint = NWEndpoint.hostPort(host: .ipv4(.loopback), port: nwPort)

let listener: NWListener
do {
    listener = try NWListener(using: parameters)
} catch {
    FileHandle.standardError.write(Data("foundation-models: could not listen on \(port): \(error)\n".utf8))
    exit(1)
}

listener.newConnectionHandler = handle
listener.start(queue: .main)
print("foundation-models: 127.0.0.1:\(port), context \(contextSize) tokens")
dispatchMain()
