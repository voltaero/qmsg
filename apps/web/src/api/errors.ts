import { Code, RpcError } from "./grpcweb";
import { tag, type AppErrorBody, type CommandError } from "./types";

/** A command the server ran and refused, from an `Outcome.error_json`. */
export class AppError extends Error {
  readonly code: string;
  readonly command: CommandError | null;
  /** The action may or may not have taken effect. */
  readonly deliveryUnknown: boolean;

  constructor(body: AppErrorBody) {
    super(body.command ? describeCommand(body.command) : body.message);
    this.name = "AppError";
    this.code = body.code;
    this.command = body.command ?? null;
    this.deliveryUnknown = body.delivery_unknown;
  }

  /** Per-input messages of an invalid-input error. */
  get issues(): Record<string, string> {
    const result: Record<string, string> = {};
    if (this.command && typeof this.command === "object" && "InvalidInput" in this.command) {
      for (const issue of this.command.InvalidInput) result[issue.input ?? ""] = issue.message;
    }
    return result;
  }
}

export function describeCommand(error: CommandError): string {
  const [name, value] = tag(error);
  switch (name) {
    case "Unsupported":
      return "Not supported";
    case "LoginRequired":
      return "Sign in first";
    case "Cancelled":
      return "Cancelled";
    case "UnknownFunction":
      return `Unknown function “${value}”`;
    case "InvalidInput": {
      const issues = value as { input?: string | null; message: string }[];
      if (!issues.length) return "Invalid input";
      return issues.map((i) => (i.input ? `${i.input}: ${i.message}` : i.message)).join("\n");
    }
    case "Provider":
      return (value as { message: string }).message;
    case "UnknownOrganization":
      return "Unknown organization";
    case "UnknownChannel":
      return "Unknown channel";
    case "UnknownUser":
      return "Unknown user";
    case "UnknownMessage":
      return "Unknown message";
    case "UnknownBlob":
      return "That file is no longer available";
    case "Rejected":
      return `Rejected: ${describeViolation(value)}`;
    case "RateLimited": {
      const ms = (value as { retry_after_ms?: number | null }).retry_after_ms;
      return ms ? `Rate limited; try again in ${Math.max(1, Math.round(ms / 1000))} s` : "Rate limited; try again later";
    }
    case "Forbidden":
      return `Not allowed: ${value}`;
    case "Failed":
      return String(value);
    default:
      return name;
  }
}

/** Readable text for a channel rule a message breaks. */
export function describeViolation(violation: unknown): string {
  if (violation === "Empty") return "the message is empty";
  if (violation && typeof violation === "object") {
    const [name, fields] = tag(violation as object) as [string, Record<string, unknown> | undefined];
    switch (name) {
      case "TextTooLong":
        return `the text is too long (${fields?.length ?? "?"} of ${fields?.max ?? "?"})`;
      case "TooLarge":
        return "a file is too large";
      case "TooManyAttachments":
        return `too many attachments (max ${fields?.max ?? "?"})`;
      case "TotalTooLarge":
        return "the files are too large together";
      case "Unsupported":
        return "this channel doesn't take that kind of content";
      case "UnsupportedMime":
        return "this channel doesn't take that file type";
      case "InvalidFormatting":
        return "invalid formatting";
      default:
        return name;
    }
  }
  return "the message breaks the channel's rules";
}

export function describe(error: unknown): string {
  if (error instanceof RpcError) {
    switch (error.code) {
      case Code.UNAVAILABLE:
        return "Can't reach the server.";
      case Code.DEADLINE_EXCEEDED:
        return "The server took too long to answer.";
      case Code.UNAUTHENTICATED:
        return "This browser is no longer paired.";
      case Code.RESOURCE_EXHAUSTED:
        return error.message || "The server is busy.";
      default:
        return error.message;
    }
  }
  if (error instanceof Error) return error.message;
  return String(error);
}

export function isUnauthenticated(error: unknown): boolean {
  return error instanceof RpcError && error.code === Code.UNAUTHENTICATED;
}
