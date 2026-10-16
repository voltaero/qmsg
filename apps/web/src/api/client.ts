import { create } from "@bufbuild/protobuf";
import * as pb from "~/gen/app_pb";
import { AppError } from "./errors";
import { Code, RpcError, Transport } from "./grpcweb";
import type {
  Command,
  Credential,
  Media,
  PairedDevice,
  ProviderCatalog,
  ProviderEvent,
  ProviderId,
  Reply,
  Snapshot,
} from "./types";

/** One event from `Watch`, with the cursor to resume after it. */
export type WatchEvent =
  | { type: "snapshot"; snapshot: Snapshot }
  /** `event` is null for an update this app doesn't understand; it still advances the cursor. */
  | { type: "update"; event: ProviderEvent | null; epoch: string; sequence: number }
  | { type: "heartbeat"; epoch: string; sequence: number };

/** A journaled action's identity. Keep it, unchanged, across retries of the same request. */
export interface ActionOperation {
  id: string;
  createdAtMs: number;
}

export function newOperation(): ActionOperation {
  return { id: crypto.randomUUID(), createdAtMs: Date.now() };
}

export type ExecuteResult = { ok: true; reply: Reply } | { ok: false; error: AppError };

export interface ProviderSettings {
  name: string;
  type: string;
  label?: string;
  enabled: boolean;
  settings: Record<string, string>;
}

/** What the app needs from a server: the gRPC client, or the offline demo. */
export interface Backend {
  readonly demo: boolean;
  setToken(token: string | undefined): void;
  pair(code: string, deviceName: string): Promise<Credential>;
  /** Replaces the token. Pass a persisted secret so a lost reply can be recovered. */
  rotate(secret: string): Promise<Credential>;
  devices(): Promise<PairedDevice[]>;
  revoke(deviceId: string): Promise<void>;
  providerCatalog(): Promise<ProviderCatalog>;
  configureProvider(settings: ProviderSettings): Promise<void>;
  removeProvider(name: string): Promise<void>;
  /** `token` overrides the client's, to probe a rotated token. */
  snapshot(token?: string): Promise<Snapshot>;
  /** Events after `(epoch, sequence)`, or a snapshot first when the server can't resume from there. */
  watch(epoch: string, after: number, signal: AbortSignal): AsyncIterable<WatchEvent>;
  execute(command: Command, provider: ProviderId, operation?: ActionOperation, timeoutMs?: number): Promise<ExecuteResult>;
  /** Uploads a file for one provider instance; returns media with a Blob source. */
  upload(provider: ProviderId, file: File): Promise<Media>;
  readBlob(id: string, maxBytes?: number): Promise<Uint8Array<ArrayBuffer>>;
  releaseBlob(id: string): Promise<void>;
}

export const CHUNK = 256 * 1024;
const MAX_UPLOAD = 100 * 1024 * 1024;

function parse<T>(json: string): T {
  return JSON.parse(json) as T;
}

export class QmsgClient implements Backend {
  readonly demo = false;
  private readonly transport: Transport;
  private token: string | undefined;

  constructor(baseUrl: string, token?: string) {
    this.transport = new Transport(baseUrl.replace(/\/$/, ""));
    this.token = token;
  }

  setToken(token: string | undefined) {
    this.token = token;
  }

  private opts(timeoutMs = 30_000, token = this.token) {
    return { token, timeoutMs };
  }

  async pair(code: string, deviceName: string): Promise<Credential> {
    const reply = await this.transport.unary(
      "Pair",
      pb.PairRequestSchema,
      pb.CredentialSchema,
      create(pb.PairRequestSchema, { code, deviceName }),
      { timeoutMs: 30_000 },
    );
    return { deviceId: reply.deviceId, token: reply.token, expiresAtMs: Number(reply.expiresAtMs) };
  }

  async rotate(secret: string): Promise<Credential> {
    const reply = await this.transport.unary(
      "Rotate",
      pb.RotateRequestSchema,
      pb.CredentialSchema,
      create(pb.RotateRequestSchema, { replacementSecret: secret }),
      this.opts(),
    );
    return { deviceId: reply.deviceId, token: reply.token, expiresAtMs: Number(reply.expiresAtMs) };
  }

  async devices(): Promise<PairedDevice[]> {
    const reply = await this.transport.unary("Devices", pb.EmptySchema, pb.DeviceListSchema, create(pb.EmptySchema), this.opts());
    return reply.devices.map((d) => ({ id: d.id, name: d.name, expiresAtMs: Number(d.expiresAtMs) }));
  }

  async revoke(deviceId: string): Promise<void> {
    await this.transport.unary("Revoke", pb.RevokeRequestSchema, pb.EmptySchema, create(pb.RevokeRequestSchema, { deviceId }), this.opts());
  }

  async providerCatalog(): Promise<ProviderCatalog> {
    const data = await this.transport.unary("ProviderCatalog", pb.EmptySchema, pb.DataSchema, create(pb.EmptySchema), this.opts());
    return parse(data.json);
  }

  async configureProvider(s: ProviderSettings): Promise<void> {
    await this.transport.unary(
      "ConfigureProvider",
      pb.ProviderConfigurationSchema,
      pb.EmptySchema,
      create(pb.ProviderConfigurationSchema, {
        name: s.name,
        type: s.type,
        label: s.label ?? "",
        enabled: s.enabled,
        settingsJson: JSON.stringify(s.settings),
      }),
      this.opts(),
    );
  }

  async removeProvider(name: string): Promise<void> {
    await this.transport.unary("RemoveProvider", pb.ProviderNameSchema, pb.EmptySchema, create(pb.ProviderNameSchema, { name }), this.opts());
  }

  async snapshot(token?: string): Promise<Snapshot> {
    const data = await this.transport.unary(
      "Snapshot",
      pb.EmptySchema,
      pb.DataSchema,
      create(pb.EmptySchema),
      this.opts(30_000, token ?? this.token),
    );
    return parse(data.json);
  }

  async *watch(epoch: string, after: number, signal: AbortSignal): AsyncGenerator<WatchEvent> {
    const stream = this.transport.serverStream(
      "Watch",
      pb.WatchRequestSchema,
      pb.EventSchema,
      create(pb.WatchRequestSchema, { epoch, afterSequence: BigInt(after) }),
      { token: this.token, signal },
    );
    for await (const event of stream) {
      const sequence = Number(event.sequence);
      switch (event.body.case) {
        case "snapshot": {
          const snapshot = parse<Snapshot>(event.body.value.json);
          yield { type: "snapshot", snapshot: { ...snapshot, epoch: event.epoch, sequence } };
          break;
        }
        case "update": {
          let update: ProviderEvent | null = null;
          try {
            update = parse(event.body.value.json);
          } catch {
            // Unknown to this app; still advances the cursor.
          }
          yield { type: "update", event: update, epoch: event.epoch, sequence };
          break;
        }
        default:
          yield { type: "heartbeat", epoch: event.epoch, sequence };
      }
    }
  }

  async execute(command: Command, provider: ProviderId, operation?: ActionOperation, timeoutMs = 60_000): Promise<ExecuteResult> {
    const bounded = Math.min(Math.max(timeoutMs, 100), 900_000);
    const outcome = await this.transport.unary(
      "Execute",
      pb.ExecuteRequestSchema,
      pb.OutcomeSchema,
      create(pb.ExecuteRequestSchema, {
        provider: { name: provider.name, instance: BigInt(provider.instance) },
        commandJson: JSON.stringify(command),
        operationId: operation?.id ?? "",
        createdAtMs: BigInt(operation?.createdAtMs ?? 0),
        timeoutMs: BigInt(bounded),
      }),
      // Give the server time to answer its own timeout first.
      this.opts(bounded + 15_000),
    );
    switch (outcome.result.case) {
      case "reply":
        return { ok: true, reply: parse(outcome.result.value.json) };
      case "errorJson":
        return { ok: false, error: new AppError(parse(outcome.result.value)) };
      default:
        throw new RpcError(Code.INTERNAL, "empty outcome");
    }
  }

  async upload(provider: ProviderId, file: File): Promise<Media> {
    if (file.size > MAX_UPLOAD) throw new RpcError(Code.RESOURCE_EXHAUSTED, "Files can be at most 100 MiB.");
    const bytes = new Uint8Array(await file.arrayBuffer());
    const chunks = [];
    for (let offset = 0; offset < bytes.length || offset === 0; offset += CHUNK) {
      const data = bytes.subarray(offset, Math.min(offset + CHUNK, bytes.length));
      chunks.push(
        offset === 0
          ? create(pb.UploadChunkSchema, {
              provider: { name: provider.name, instance: BigInt(provider.instance) },
              name: file.name,
              mime: file.type || "application/octet-stream",
              size: BigInt(bytes.length),
              data,
            })
          : create(pb.UploadChunkSchema, { data }),
      );
      if (!bytes.length) break;
    }
    const data = await this.transport.clientStream("Upload", pb.UploadChunkSchema, pb.DataSchema, chunks, this.opts(130_000));
    return parse(data.json);
  }

  async readBlob(id: string, maxBytes = 100 * 1024 * 1024): Promise<Uint8Array<ArrayBuffer>> {
    const parts: Uint8Array[] = [];
    let total = 0;
    for (;;) {
      const chunk = await this.transport.unary(
        "ReadBlob",
        pb.ReadBlobRequestSchema,
        pb.BlobChunkSchema,
        create(pb.ReadBlobRequestSchema, { id, offset: BigInt(total), length: CHUNK }),
        this.opts(70_000),
      );
      parts.push(chunk.data);
      total += chunk.data.length;
      if (chunk.data.length < CHUNK) break;
      if (total > maxBytes) throw new RpcError(Code.RESOURCE_EXHAUSTED, "file too large");
    }
    const result = new Uint8Array(total);
    let offset = 0;
    for (const part of parts) {
      result.set(part, offset);
      offset += part.length;
    }
    return result;
  }

  async releaseBlob(id: string): Promise<void> {
    await this.transport.unary("ReleaseBlob", pb.ReleaseBlobRequestSchema, pb.EmptySchema, create(pb.ReleaseBlobRequestSchema, { id }), this.opts());
  }
}
