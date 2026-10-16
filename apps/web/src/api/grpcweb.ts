// A small gRPC-Web client over fetch. The orchestrator speaks gRPC-Web
// through tonic-web, in binary mode. A client stream is sent as one request
// body of several frames, since fetch can't stream a request; that suits
// uploads, which are bounded anyway.

import { fromBinary, toBinary, type DescMessage, type MessageShape } from "@bufbuild/protobuf";

export const Code = {
  OK: 0,
  CANCELLED: 1,
  UNKNOWN: 2,
  INVALID_ARGUMENT: 3,
  DEADLINE_EXCEEDED: 4,
  NOT_FOUND: 5,
  ALREADY_EXISTS: 6,
  PERMISSION_DENIED: 7,
  RESOURCE_EXHAUSTED: 8,
  FAILED_PRECONDITION: 9,
  ABORTED: 10,
  OUT_OF_RANGE: 11,
  UNIMPLEMENTED: 12,
  INTERNAL: 13,
  UNAVAILABLE: 14,
  DATA_LOSS: 15,
  UNAUTHENTICATED: 16,
} as const;
export type Code = (typeof Code)[keyof typeof Code];

const codeNames = Object.fromEntries(Object.entries(Code).map(([k, v]) => [v, k])) as Record<number, string>;

export class RpcError extends Error {
  constructor(
    readonly code: Code,
    message: string,
  ) {
    super(message || codeNames[code] || `gRPC status ${code}`);
    this.name = "RpcError";
  }
}

export interface CallOptions {
  token?: string;
  /** Milliseconds; sent as grpc-timeout and enforced locally. */
  timeoutMs?: number;
  signal?: AbortSignal;
}

const SERVICE = "qmsg.app.v1.App";

function frame(payloads: Uint8Array[]): Uint8Array<ArrayBuffer> {
  const size = payloads.reduce((n, p) => n + 5 + p.length, 0);
  const body = new Uint8Array(size);
  let offset = 0;
  for (const payload of payloads) {
    body[offset] = 0;
    new DataView(body.buffer).setUint32(offset + 1, payload.length);
    body.set(payload, offset + 5);
    offset += 5 + payload.length;
  }
  return body;
}

function headers(options: CallOptions): Headers {
  const h = new Headers({
    "content-type": "application/grpc-web+proto",
    "x-grpc-web": "1",
    accept: "application/grpc-web+proto",
  });
  if (options.token) h.set("authorization", `Bearer ${options.token}`);
  if (options.timeoutMs) h.set("grpc-timeout", `${Math.ceil(options.timeoutMs)}m`);
  return h;
}

function statusFrom(fields: Map<string, string>): RpcError | null {
  const raw = fields.get("grpc-status");
  if (raw === undefined) return null;
  const code = Number(raw) as Code;
  if (code === Code.OK) return null;
  let message = fields.get("grpc-message") ?? "";
  try {
    message = decodeURIComponent(message);
  } catch {
    // Keep it as sent.
  }
  return new RpcError(code, message);
}

function parseTrailers(bytes: Uint8Array): Map<string, string> {
  const fields = new Map<string, string>();
  for (const line of new TextDecoder().decode(bytes).split("\r\n")) {
    const colon = line.indexOf(":");
    if (colon > 0) fields.set(line.slice(0, colon).trim().toLowerCase(), line.slice(colon + 1).trim());
  }
  return fields;
}

function httpError(status: number): RpcError {
  switch (status) {
    case 400:
      return new RpcError(Code.INTERNAL, "The server didn't understand the request.");
    case 401:
      return new RpcError(Code.UNAUTHENTICATED, "");
    case 403:
      return new RpcError(Code.PERMISSION_DENIED, "");
    case 404:
      return new RpcError(Code.UNIMPLEMENTED, "This server has no qmsg app API here.");
    case 429:
    case 502:
    case 503:
    case 504:
      return new RpcError(Code.UNAVAILABLE, "");
    default:
      return new RpcError(Code.UNKNOWN, `HTTP ${status}`);
  }
}

/** Splits a response body into messages, then checks its trailers. */
async function* frames(response: Response): AsyncGenerator<Uint8Array> {
  const reader = response.body?.getReader();
  if (!reader) throw new RpcError(Code.INTERNAL, "empty response");
  let buffer = new Uint8Array(0);
  let trailers: Map<string, string> | null = null;
  try {
    for (;;) {
      while (buffer.length >= 5) {
        const length = new DataView(buffer.buffer, buffer.byteOffset).getUint32(1);
        if (buffer.length < 5 + length) break;
        const flag = buffer[0];
        const payload = buffer.slice(5, 5 + length);
        buffer = buffer.slice(5 + length);
        if (flag & 0x80) trailers = parseTrailers(payload);
        else yield payload;
      }
      const { done, value } = await reader.read();
      if (done) break;
      const next = new Uint8Array(buffer.length + value.length);
      next.set(buffer);
      next.set(value, buffer.length);
      buffer = next;
    }
  } finally {
    reader.releaseLock();
  }
  if (trailers) {
    const error = statusFrom(trailers);
    if (error) throw error;
  } else if (buffer.length) {
    throw new RpcError(Code.INTERNAL, "truncated response");
  } else {
    throw new RpcError(Code.UNAVAILABLE, "The connection closed early.");
  }
}

export class Transport {
  constructor(readonly baseUrl: string) {}

  private async request(method: string, body: Uint8Array<ArrayBuffer>, options: CallOptions): Promise<Response> {
    const signals = [options.signal, options.timeoutMs ? AbortSignal.timeout(options.timeoutMs + 5_000) : undefined];
    const signal = AbortSignal.any(signals.filter((s): s is AbortSignal => !!s));
    let response: Response;
    try {
      response = await fetch(`${this.baseUrl}/${SERVICE}/${method}`, {
        method: "POST",
        headers: headers(options),
        body,
        signal,
        cache: "no-store",
      });
    } catch (error) {
      if (options.signal?.aborted) throw new RpcError(Code.CANCELLED, "");
      if (signal.aborted) throw new RpcError(Code.DEADLINE_EXCEEDED, "");
      throw new RpcError(Code.UNAVAILABLE, error instanceof Error ? error.message : "");
    }
    if (!response.ok) throw httpError(response.status);
    // A trailers-only response puts its status in the headers.
    const error = statusFrom(new Map(response.headers.entries()));
    if (error) throw error;
    return response;
  }

  async unary<I extends DescMessage, O extends DescMessage>(
    method: string,
    input: I,
    output: O,
    message: MessageShape<I>,
    options: CallOptions = {},
  ): Promise<MessageShape<O>> {
    return this.clientStream(method, input, output, [message], options);
  }

  /** Sends every message in one body; for bounded client streams. */
  async clientStream<I extends DescMessage, O extends DescMessage>(
    method: string,
    input: I,
    output: O,
    messages: MessageShape<I>[],
    options: CallOptions = {},
  ): Promise<MessageShape<O>> {
    const response = await this.request(method, frame(messages.map((m) => toBinary(input, m))), options);
    let result: MessageShape<O> | undefined;
    for await (const payload of frames(response)) {
      result ??= fromBinary(output, payload);
    }
    if (!result) throw new RpcError(Code.INTERNAL, "no response message");
    return result;
  }

  async *serverStream<I extends DescMessage, O extends DescMessage>(
    method: string,
    input: I,
    output: O,
    message: MessageShape<I>,
    options: CallOptions = {},
  ): AsyncGenerator<MessageShape<O>> {
    const response = await this.request(method, frame([toBinary(input, message)]), options);
    try {
      for await (const payload of frames(response)) yield fromBinary(output, payload);
    } catch (error) {
      if (options.signal?.aborted) throw new RpcError(Code.CANCELLED, "");
      if (error instanceof RpcError) throw error;
      throw new RpcError(Code.UNAVAILABLE, error instanceof Error ? error.message : "");
    }
  }
}
