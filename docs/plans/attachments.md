# Plan: attachments

A prompt sent through Ren carries attachments: images first, then other files.
Ren has one attachment vocabulary across the UI, the control API, the
supervisor, and storage; each client declares, as data, which kinds it
accepts and how it takes them, and only its codec turns a Ren attachment into
the client's own input. Nothing in this plan is built. Today a `Prompt`
(`ren_harness::drive`) is non-blank text, and each codec sends it as one text
item. Record every deviation here, with its reason.

## Decisions

- **Images first.** PNG, JPEG, GIF, and WebP, sent so the model sees them:
  every client Ren drives takes images natively in its structured mode.
  Screenshots are the case to serve (a broken view, an error dialog, a
  Redline mark).
- **Files second, by path.** Logs, CSV, JSON, patches, and PDFs are saved and
  named in the prompt; the agent reads them with its own tools. This works
  alike for every client and every format. PDFs are files, not native
  documents, even for the clients that could take them. A file already in
  the run's folder stays an `@` mention and is never uploaded.
- **Out of scope:** audio, video, archives, executables, SVG (it can carry
  script, and the provider APIs refuse it), and URLs Ren would fetch.
- **One vocabulary, declared per client.** Ren's types are the only ones the
  UI, API, supervisor, and store see. Each client's acceptance is data
  returned by a `match` on `Harness`; there is no per-client trait
  (`AGENTS.md`, "Harnesses").
- **Refuse, never drop.** A prompt whose attachment its client or model
  cannot take is refused with a typed error before anything is sent. An
  attachment is never silently left out of a prompt that then runs.
- **Stored as files, scoped by operator session**, in Ren's data folder,
  never in the run's folder. Path delivery (Codex's local images, and files)
  needs a real path, so the file on disk is the source and the store keeps
  its metadata.
- **Uploaded apart from the prompt.** An upload returns an `AttachmentId`;
  a prompt names its attachments by id. Prompt bodies stay small, and a
  queued or steering prompt carries attachments like a first one.

## Model

- `AttachmentId` (`ren_recorder::id`): Ren's identity for one uploaded
  attachment, from the `IdSource`.
- `Attachment` (`ren_harness`, a new `attachment` module): its id, its
  `AttachmentKind`, its byte size, its SHA-256, and its display name (a
  sanitized file name, never a path).
- `AttachmentKind`: `Image(ImageFormat)`, and in phase 4 `File`.
  `ImageFormat` is a vocabulary (`png`, `jpeg`, `gif`, `webp`) with labels
  pinned in `ren-harness`'s `tests/fixtures/labels.txt`.
- **Parsing** is pure: `Attachment::parse` reads the leading bytes for the
  format (never the name or the browser's media type) and the header for
  the image's dimensions, and refuses anything else with an
  `AttachmentError`. Past it, an attachment is known to be what it says.
- `Prompt` gains its attachments: its text and an ordered list of
  `Attachment`s. A prompt of attachments only is allowed; one with neither
  text nor attachments is still blank.
- `Acceptance`: what a client takes, as data. For each kind, whether it is
  accepted and its `Delivery`, plus the client's own limits:
  - `InlineBase64`: the bytes, encoded, in the message (Claude Code, Pi,
    OpenCode).
  - `LocalPath`: the file's path in a native input item (Codex's
    `localImage`).
  - `MentionPath`: the path named in the prompt's text (files, every client).
- **Static and learned.** `Harness::acceptance` is the static declaration.
  What only the running client says is learned by its codec and reported to
  the supervisor: OpenCode's `promptCapabilities.image` in its `initialize`
  answer, and whether the chosen model reads images where the client's model
  listing says so (`Model` gains its input kinds when the listing has them).
  The session's effective acceptance is the static declaration narrowed by
  what was learned; unknown is not yes.
- `Limits`: declarative configuration (`[attachments]`), with defaults: 5 MB
  and 8000 px a side per image (the Anthropic API's limits), 10 attachments
  and 20 MB per prompt, and in phase 4, 25 MB per file. Inline delivery
  grows bytes by a third (base64); limits are checked on the stored size and
  the encoded size both.
- Errors: `AttachmentError` (unreadable, unsupported format, too large, too
  many), and the supervisor's refusals: not accepted by this client, not by
  this model, not this session's, gone.

## Storage

- Layout, under the daemon's data directory:

  ```
  <data>/attachments/<operator-session-id>/<attachment-id>/<name>
  ```

  An upload is written to `<data>/attachments/.incoming/` and renamed into
  place once it is complete and parsed; a partial upload never appears in a
  session's folder.
- **Scoped by session.** Deleting a session removes its folder; archiving
  keeps it; retention acts on a session's folder as a whole. A client is
  given its own session's folder only (as an extra directory it may read,
  phase 3), never `attachments/`, so one session's agent cannot read
  another's screenshots.
- **Metadata** goes in a new `ren.sqlite3` table (a new migration): id,
  operator session, kind, size, hash, name, and upload time. A run entry
  for a prompt (`run_entries`) names its attachments by id; image bytes
  never pass through the recorder queue, whose byte budget they would
  overrun.
- **The wire keeps a second copy.** An inline image is part of the provider
  request the gateway records. Capture policy and retention apply to it as
  to any body; the plan does not strip it.
- **Secrets.** `SecretScanner` cannot read pixels, and screenshots show
  tokens. Attachments are stored as uploaded, and the UI says so. Phone
  photos carry location in their EXIF data; whether to strip it (a
  re-encode) or warn is decided in phase 2.

## Where it lives

| Piece                                                                                                       | Crate            |
| ----------------------------------------------------------------------------------------------------------- | ---------------- |
| `AttachmentId`                                                                                              | `ren-recorder`   |
| `Attachment`, `AttachmentKind`, `ImageFormat`, parsing, `Acceptance`, `Delivery`, and each codec's encoding | `ren-harness`    |
| The metadata table and its reads                                                                            | `ren-store`      |
| Session folders made, written, and removed through `AgentHost`, with the session's lifecycle                | `ren-supervisor` |
| The upload route and prompts by attachment id                                                               | `ren-gateway`    |
| The disk work behind `AgentHost`                                                                            | `ren-daemon`     |
| The composer (paste, drop, picker, chips) and thumbnails                                                    | `ui`             |

`attachment` in `ren-harness` is pure and joins its purity test. The
supervisor's session-folder bookkeeping is pure in `session`; the writes go
through the `AgentHost` port, which the tests fill in memory.

## API

- `POST /control/v1/sessions/{id}/attachments`: the raw bytes, with the
  file name in a header; a byte cap from `Limits`, enforced while reading.
  Answers the `Attachment`, or a typed refusal.
- `GET /control/v1/sessions/{id}/attachments/{attachment}`: the bytes, for
  the UI's thumbnails. Non-cacheable, `Content-Type` from the parsed format,
  `Content-Disposition: attachment` for files, and `nosniff`.
- `DELETE` of one not yet sent in a prompt (a chip removed from the
  composer). One sent stays with the session.
- The prompt request (`PromptRequest`) gains `attachments: [AttachmentId]`.
- A session's view gains its effective `acceptance`, so the composer offers
  only what the client takes. The server checks again on send.
- The UI's `RunsProxy` (`ui/src/lib/server/control.ts`) forwards the upload
  and the read with the control token, like the other session routes.

## Clients

What each client takes, to confirm against its current documentation before
its codec is written (`AGENTS.md`: check official documentation before
changing client behavior):

| Client      | Image input                                                                        | Mid-turn steer with an image |
| ----------- | ---------------------------------------------------------------------------------- | ---------------------------- |
| Codex       | `localImage` (a path) or `image` input items in `turn/start`                       | to confirm (`turn/steer`)    |
| Claude Code | `image` content blocks (base64) in a stream-json user message                      | to confirm                   |
| Pi          | `images` on the RPC `prompt` command                                               | to confirm (`steer`)         |
| OpenCode    | an ACP `image` content block, when `initialize` reports `promptCapabilities.image` | to confirm                   |

Each codec's encoding is tested like the rest of the codec: a prompt with
attachments in, the client's lines out, no I/O. Where a steer cannot carry
an image, `Midway` says so and a steering prompt with one is refused
(queued instead, if the operator chooses).

Reading access for files (phase 4): Claude Code's `--add-dir`, Codex's
sandbox readable roots, and the equivalents for Pi and OpenCode, given the
session's folder only. To confirm per client.

## UI

- The composer (`ui/src/routes/sessions/+page.svelte`) takes a pasted image,
  a dropped file, or a picked one. Each upload is a chip with its thumbnail
  or name, its progress, and a remove control; sending waits for uploads to
  finish. What the session's acceptance refuses is greyed with the reason.
- The conversation shows a prompt's images as thumbnails, opened full size
  on click, and its files by name.
- ↑ recalls an earlier prompt with its attachments.
- Redline's brief can attach its screenshot (phase 5).
- Pure logic (acceptance checks, chip states as a discriminated union) in
  `ui/src/lib`, tested without a browser. Thumbnails are served same-origin;
  `ui/src/csp.ts` needs no new source.

## Phases

1. **Vocabulary and parsing.** `AttachmentId`, `Attachment`, `ImageFormat`,
   `Attachment::parse`, `Limits`, `Acceptance` with each client's static
   declaration, and `Prompt` with attachments (none sent yet). Unit tests,
   labels pinned. Not built.
2. **Storage and upload.** The migration, the session folder under
   `AgentHost`, the upload, read, and delete routes, deletion with the
   session, and the EXIF decision. Supervisor and `TestGateway` tests with
   in-memory hosts. Not built.
3. **Images through every client.** Each codec's encoding, the learned
   acceptance (OpenCode's capabilities, models' input kinds), refusals on
   send, run entries naming attachments, and the UI composer and
   conversation. Fake clients in `cargo xtask dev` take images, so the dev
   slot shows them. Not built.
4. **Files.** `AttachmentKind::File`, `MentionPath` delivery, each client
   given its session's folder to read, and the UI's file chips. Not built.
5. **Redline screenshots.** A Redline brief sends its marked screenshot as
   an image. Not built.

## Deviations

None yet.
