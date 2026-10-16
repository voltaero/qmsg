// The qmsg JSON the app API carries, as the Rust types serialize it. Enums
// use Serde's external tag: a unit variant is a bare string such as "Ready",
// any other variant an object with one key, such as {"Failed": "reason"}.
// See crates/qmsg-orchestrator/APP_API.md and proto/schema.

/** The variant name and payload of an externally tagged enum value. */
export function tag<T extends string | object>(value: T): [string, unknown] {
  if (typeof value === "string") return [value, undefined];
  const [name] = Object.keys(value);
  return [name!, (value as Record<string, unknown>)[name!]];
}

export interface ChannelRef {
  organization: string | null;
  channel: string;
}

export interface ProviderId {
  name: string;
  instance: number;
}

export interface Message {
  id: string;
  channel: ChannelRef;
  author: string;
  /** Milliseconds since the Unix epoch. */
  sent_at: number;
  reply_to?: string | null;
  content: Content[];
  edited_at?: number | null;
  reactions: Reaction[];
  nonce?: string | null;
  email?: Email | null;
}

export interface EmailAddress {
  address: string;
  name?: string | null;
}

export interface Email {
  subject: string;
  from: EmailAddress[];
  to: EmailAddress[];
  cc: EmailAddress[];
  bcc: EmailAddress[];
  reply_to: EmailAddress[];
  message_id?: string | null;
  in_reply_to?: string | null;
  references: string[];
  /** Untrusted; never render without sanitizing. */
  html?: string | null;
  /** Content index, as a string, to its Content-ID. */
  content_ids: Record<string, string>;
  read: boolean;
  starred: boolean;
  draft: boolean;
}

export interface Mailbox {
  delimiter?: string | null;
  attributes: string[];
}

export type Content =
  | { Text: string }
  | { Formatted: FormattedText }
  | { Image: Media }
  | { Video: Media }
  | { Audio: Media }
  | { File: Media }
  | { Custom: { kind: string; data: number[] } };

export type MediaKind = "Image" | "Video" | "Audio" | "File";

export interface Media {
  name?: string | null;
  mime?: string | null;
  size?: number | null;
  source: MediaSource;
  metadata?: MediaMetadata;
}

export interface MediaMetadata {
  width?: number | null;
  height?: number | null;
  duration_ms?: number | null;
  expires_at_ms?: number | null;
  preview_url?: string | null;
  description?: string | null;
  spoiler?: boolean;
}

export type MediaSource = { Bytes: number[] } | { Url: string } | { Blob: string };

export type ContentKind = "Text" | "Formatted" | "Image" | "Video" | "Audio" | "File" | { Custom: string };

export type TextUnit = "Chars" | "Utf16" | "Bytes";

export interface ContentRule {
  kind: ContentKind;
  max_size?: number | null;
  text_unit: TextUnit;
  mime_types: string[];
}

export interface MessageLimits {
  max_attachments?: number | null;
  max_total_size?: number | null;
}

export interface User {
  id: string;
  name: string;
  username?: string | null;
  avatar?: Media | null;
}

export type ChannelKind =
  | "Text"
  | "Voice"
  | "Video"
  | "Announcement"
  | "Forum"
  | "Thread"
  | "Category"
  | "Direct"
  | "Group"
  | { Custom: string };

export interface Channel {
  id: string;
  name: string;
  topic?: string | null;
  icon?: Media | null;
  kind: ChannelKind;
  parent?: string | null;
  position?: number | null;
  members?: string[] | null;
  accepted_content: ContentRule[];
  limits: MessageLimits;
  mailbox?: Mailbox | null;
}

export type Presence = "Online" | "Idle" | "Busy" | "Offline";

/** The users and channels of an organization, or outside any; keyed by id. */
export interface Scope {
  me?: string | null;
  users: Record<string, User>;
  channels: Record<string, Channel>;
  presence: Record<string, Presence>;
}

export interface OrganizationEntry {
  id: string;
  name: string;
  icon?: Media | null;
  scope: Scope;
}

export interface Organization {
  id: string;
  name: string;
  icon?: Media | null;
  me?: string | null;
  users: User[];
  channels: Channel[];
}

export interface LoginStep {
  message: string;
  /** A login function in the provider context to fill in and call. */
  function?: string | null;
  /** A page to open in a browser. */
  link?: string | null;
  /** Text to show as a QR code for the platform's phone app. */
  qr_code?: string | null;
}

export type ProviderStatus =
  | "Connecting"
  | "Syncing"
  | "Ready"
  | { LoginRequired: LoginStep }
  | { Disconnected: { reason: string } }
  | { Failed: string };

export interface FormattedText {
  text: string;
  /** Byte ranges into the UTF-8 text. */
  spans: Span[];
}

export interface Span {
  start: number;
  end: number;
  style: Style;
}

export type Mention = { User: string } | { Channel: string } | { Role: string } | "Everyone";

export type Style =
  | "Bold"
  | "Italic"
  | "Underline"
  | "Strikethrough"
  | "Code"
  | "Quote"
  | "Spoiler"
  | { CodeBlock: { language?: string | null } }
  | { Link: string }
  | { Mention: Mention };

export interface Reaction {
  key: string;
  count: number;
  me: boolean;
}

// MARK: Events

export type MessageEvent =
  | { Received: Message }
  | { Edited: { channel: ChannelRef; id: string; content: Content[]; edited_at: number } }
  | { Deleted: { channel: ChannelRef; id: string } }
  | { Reacted: { channel: ChannelRef; id: string; user: string; key: string; added: boolean } }
  | { Read: { channel: ChannelRef; user: string; up_to?: string | null } }
  | { Typing: { channel: ChannelRef; user: string } }
  | { Gap: { channel: ChannelRef } }
  | { NotSent: { channel: ChannelRef; nonce: string; error: CommandError } };

export function eventChannel(event: MessageEvent): ChannelRef {
  if ("Received" in event) return event.Received.channel;
  const [, payload] = tag(event);
  return (payload as { channel: ChannelRef }).channel;
}

export type DirectoryUpdate =
  | "Reset"
  | { OrganizationUpserted: Organization }
  | { OrganizationUpdated: { id: string; name: string; icon?: Media | null } }
  | { OrganizationRemoved: { id: string } }
  | { UserUpserted: { organization?: string | null; user: User } }
  | { UserRemoved: { organization?: string | null; id: string } }
  | { ChannelUpserted: { organization?: string | null; channel: Channel } }
  | { ChannelRemoved: ChannelRef }
  | { Me: { organization?: string | null; id?: string | null } }
  | { Presence: { organization?: string | null; user: string; presence: Presence } };

export type ProviderEvent =
  | { Message: { provider: ProviderId; event: MessageEvent } }
  | { Directory: { provider: ProviderId; update: DirectoryUpdate } }
  | { Status: { provider: ProviderId; status: ProviderStatus } }
  | { Exited: { provider: ProviderId; result: { Ok: null } | { Err: string } } };

export type CommandError =
  | "Unsupported"
  | "LoginRequired"
  | "Cancelled"
  | { UnknownFunction: string }
  | { InvalidInput: InputIssue[] }
  | { Provider: { code: string; message: string; details?: Value | null } }
  | { UnknownOrganization: string }
  | { UnknownChannel: ChannelRef }
  | { UnknownUser: string }
  | { UnknownMessage: string }
  | { UnknownBlob: string }
  | { Rejected: unknown }
  | { RateLimited: { retry_after_ms?: number | null } }
  | { Forbidden: string }
  | { Failed: string };

/** The `error_json` of an `Outcome`. */
export interface AppErrorBody {
  code: string;
  message: string;
  command?: CommandError | null;
  /** The action may or may not have taken effect. */
  delivery_unknown: boolean;
}

// MARK: Functions

export type Value =
  | "Null"
  | { Boolean: boolean }
  | { Integer: number }
  | { Text: string }
  | { Bytes: number[] }
  | { List: Value[] }
  | { Record: Record<string, Value> }
  | { Content: Content[] }
  | { Channel: ChannelRef }
  | { Message: Message };

export type Arguments = Record<string, Value>;

export type ValueType =
  | "Any"
  | "Null"
  | "Boolean"
  | "Integer"
  | "Text"
  | "Bytes"
  | "Content"
  | "Channel"
  | "Message"
  | { List: ValueType }
  | { Record: Field[] };

export interface Field {
  id: string;
  label: string;
  value_type: ValueType;
  required: boolean;
}

export type FunctionContext =
  | "Provider"
  | { Organization: string }
  | { Channel: ChannelRef }
  | { Message: { channel: ChannelRef; id: string } };

export type ActionKind =
  | "SendMessage"
  | "EditMessage"
  | "DeleteMessage"
  | "OpenChannel"
  | "AddReaction"
  | "RemoveReaction"
  | "MarkRead"
  | "MarkUnread"
  | "Typing"
  | "Login"
  | "Logout"
  | { Custom: string };

export type FunctionKind =
  | { Action: { action: ActionKind; result: ValueType } }
  | { Lookup: ValueType }
  | { Verification: ValueType }
  | { Completion: ValueType };

export interface Choice {
  label: string;
  value: Value;
}

export type InputHint = "Plain" | "Secret" | "Multiline" | { Choices: Choice[] };

export interface FunctionInput {
  id: string;
  label: string;
  description: string;
  value_type: ValueType;
  required: boolean;
  hint: InputHint;
  verification?: string | null;
  completion?: string | null;
}

export interface QFunction {
  id: string;
  label: string;
  description: string;
  kind: FunctionKind;
  inputs: FunctionInput[];
}

export function actionOf(f: QFunction): ActionKind | undefined {
  return "Action" in f.kind ? f.kind.Action.action : undefined;
}

export interface FunctionCall {
  function: string;
  context: FunctionContext;
  arguments: Arguments;
}

export interface FunctionInputRef {
  function: string;
  input: string;
}

export interface InputIssue {
  input?: string | null;
  code: string;
  message: string;
}

export type Verification = "Valid" | { Invalid: InputIssue[] };

export interface CompletionItem {
  label: string;
  description?: string | null;
  value: Value;
}

export interface CompletionPage {
  items: CompletionItem[];
  next_cursor?: string | null;
}

// MARK: Commands and replies

export type Command =
  | { Functions: { context: FunctionContext } }
  | { Call: FunctionCall }
  | { Verify: { call: FunctionCall; input: FunctionInputRef | null; value: Value } }
  | {
      Complete: {
        call: FunctionCall;
        input: FunctionInputRef | null;
        query: string;
        cursor: string | null;
        limit: number;
      };
    }
  /** `limit` must be 1..=100. Pages sort oldest first; the cursor asks for older ones. */
  | { History: { channel: ChannelRef; cursor: string | null; limit: number } }
  | { GetMessage: { channel: ChannelRef; id: string } }
  | { Send: { channel: ChannelRef; reply_to: string | null; content: Content[]; nonce: string | null } }
  | { OpenChannel: { organization: string | null; members: string[] } };

export interface HistoryPage {
  /** Oldest first. */
  messages: Message[];
  /** For older messages; null at the start of the channel. */
  next_cursor?: string | null;
}

export type Reply =
  | { Functions: QFunction[] }
  | { Value: Value }
  | { Verified: Verification }
  | { Completed: CompletionPage }
  | { Sent: { id: string } }
  | { Opened: ChannelRef }
  | { History: HistoryPage }
  | { Message: Message }
  | { Blob: number[] };

export interface ProviderView {
  provider: ProviderId;
  status: ProviderStatus;
  organizations: OrganizationEntry[];
  standalone?: Scope | null;
}

export interface Snapshot {
  providers: ProviderView[];
  epoch: string;
  sequence: number;
}

// MARK: Accounts

export interface ProviderCatalog {
  types: ProviderType[];
  accounts: ManagedProvider[];
}

export interface ProviderType {
  id: string;
  label: string;
  fields: ProviderField[];
  defaults: Record<string, string>;
}

export interface ProviderField {
  id: string;
  label: string;
  required: boolean;
  secret: boolean;
}

export interface ManagedProvider {
  name: string;
  label: string;
  type: string;
  enabled: boolean;
  settings: Record<string, string>;
}

export interface PairedDevice {
  id: string;
  name: string;
  expiresAtMs: number;
}

export interface Credential {
  deviceId: string;
  token: string;
  expiresAtMs: number;
}

/** What `qmsg-orchestrator pair --json` prints. */
export interface PairingPayload {
  version: number;
  endpoint: string;
  code: string;
  expires_at_ms: number;
  certificate_pem?: string | null;
  certificate_sha256?: string | null;
}
