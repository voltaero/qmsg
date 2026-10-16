import { create, toBinary } from "@bufbuild/protobuf";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Code, RpcError, Transport } from "~/api/grpcweb";
import * as pb from "~/gen/app_pb";

function frame(flag: number, payload: Uint8Array): Uint8Array {
  const out = new Uint8Array(5 + payload.length);
  out[0] = flag;
  new DataView(out.buffer).setUint32(1, payload.length);
  out.set(payload, 5);
  return out;
}

function respond(chunks: Uint8Array[], headers: Record<string, string> = {}) {
  const body = new ReadableStream<Uint8Array>({
    start(c) {
      chunks.forEach((x) => c.enqueue(x));
      c.close();
    },
  });
  return new Response(body, { status: 200, headers: { "content-type": "application/grpc-web+proto", ...headers } });
}

const trailers = (text: string) => frame(0x80, new TextEncoder().encode(text));

afterEach(() => vi.restoreAllMocks());

describe("grpc-web", () => {
  it("frames requests and reads a message split across chunks", async () => {
    const reply = toBinary(pb.DataSchema, create(pb.DataSchema, { json: '{"ok":true}' }));
    const whole = new Uint8Array([...frame(0, reply), ...trailers("grpc-status: 0\r\n")]);
    const fetch = vi.spyOn(globalThis, "fetch").mockResolvedValue(respond([whole.slice(0, 3), whole.slice(3, 9), whole.slice(9)]));
    const result = await new Transport("http://h").unary("Snapshot", pb.EmptySchema, pb.DataSchema, create(pb.EmptySchema), { token: "t" });
    expect(result.json).toBe('{"ok":true}');
    const [url, init] = fetch.mock.calls[0]!;
    expect(url).toBe("http://h/qmsg.app.v1.App/Snapshot");
    expect((init!.headers as Headers).get("authorization")).toBe("Bearer t");
    expect(Array.from(init!.body as Uint8Array)).toEqual([0, 0, 0, 0, 0]);
  });

  it("sends a client stream as several frames in one body", async () => {
    const reply = toBinary(pb.DataSchema, create(pb.DataSchema, { json: "{}" }));
    const fetch = vi.spyOn(globalThis, "fetch").mockResolvedValue(respond([frame(0, reply), trailers("grpc-status: 0\r\n")]));
    const chunks = [create(pb.UploadChunkSchema, { name: "a", data: new Uint8Array([1, 2]) }), create(pb.UploadChunkSchema, { data: new Uint8Array([3]) })];
    await new Transport("").clientStream("Upload", pb.UploadChunkSchema, pb.DataSchema, chunks);
    const body = fetch.mock.calls[0]![1]!.body as Uint8Array;
    const first = new DataView(body.buffer).getUint32(1);
    expect(body[5 + first]).toBe(0);
    expect(body.length).toBe(5 + first + 5 + toBinary(pb.UploadChunkSchema, chunks[1]!).length);
  });

  it("reports trailer and trailers-only errors with their codes", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(respond([trailers("grpc-status: 16\r\ngrpc-message: token%20expired\r\n")]));
    await expect(new Transport("").unary("Snapshot", pb.EmptySchema, pb.DataSchema, create(pb.EmptySchema))).rejects.toEqual(
      new RpcError(Code.UNAUTHENTICATED, "token expired"),
    );
    vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(respond([], { "grpc-status": "8", "grpc-message": "busy" }));
    await expect(new Transport("").unary("Snapshot", pb.EmptySchema, pb.DataSchema, create(pb.EmptySchema))).rejects.toMatchObject({
      code: Code.RESOURCE_EXHAUSTED,
    });
  });

  it("streams messages until the trailers", async () => {
    const event = (sequence: bigint) =>
      frame(0, toBinary(pb.EventSchema, create(pb.EventSchema, { epoch: "e", sequence, body: { case: "heartbeat", value: true } })));
    vi.spyOn(globalThis, "fetch").mockResolvedValue(respond([event(1n), event(2n), trailers("grpc-status: 0\r\n")]));
    const seen: bigint[] = [];
    for await (const e of new Transport("").serverStream("Watch", pb.WatchRequestSchema, pb.EventSchema, create(pb.WatchRequestSchema))) {
      seen.push(e.sequence);
    }
    expect(seen).toEqual([1n, 2n]);
  });
});
