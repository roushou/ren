# Plan: attachments

A prompt sent through Ren carries attachments: images first, then other files.
Ren has one attachment vocabulary across the UI, the control API, the
supervisor, and storage; each client declares, as data, which kinds it
accepts and how it takes them, and only its codec turns a Ren attachment into
the client's own input. Phases 1 and 2 are built: the vocabulary, parsing,
limits, and each client's declaration (`ren_harness::attachment`), and
images uploaded to a session, kept, read back, removed, and deleted with it.
Nothing is sent yet: a `Prompt` (`ren_harness::drive`) is still non-blank
text, and each codec sends it as one text item. Record every deviation here,
with its reason.

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
  attachment, from the `IdSource`. Built.
- `Attachment` (`ren_harness::attachment`): its `AttachmentKind`, its
  `ByteSize`, its SHA-256 (`ren_depot::source::Sha256`), and its
  `AttachmentName` (the last part of the uploaded name, without control
  characters; never a path, `.`, `..`, empty, or over 255 bytes). It has
  no id: the store pairs an `AttachmentId` with it (phase 2). Built.
- `AttachmentKind`: `Image(Image)`, an `ImageFormat` and its `Dimensions`
  (never zero on a side); `File` comes in phase 4. `ImageFormat` is a
  vocabulary (`png`, `jpeg`, `gif`, `webp`) with labels pinned in
  `ren-harness`'s `tests/fixtures/labels.txt`. Built.
- **Parsing** is pure: `Attachment::parse` reads the leading bytes for the
  format (never the name or the browser's media type) and the header for
  the image's dimensions, and refuses anything else with an
  `AttachmentError`. Past it, an attachment is known to be what it says.
  Dimensions come from the `imagesize` crate (pure, no dependencies of its
  own); the image is not decoded. Built.
- `Prompt` gains its attachments: its text and an ordered list of
  attachments. A prompt of attachments only is allowed; one with neither
  text nor attachments is still blank. Phase 3.
- `Acceptance`: what a client takes, as data. For each kind, a `Takes`:
  `Always(Delivery)`, `IfOffered(Delivery)`, or `Never`. Built, with each
  client's declaration in `Harness::acceptance`; `Delivery` labels are
  pinned:
  - `InlineBase64`: the bytes, encoded, in the message (Claude Code, Pi,
    OpenCode).
  - `LocalPath`: the file's path in a native input item (Codex's
    `localImage`).
  - `MentionPath`: the path named in the prompt's text (files, every
    client). Phase 4.
- **Static and learned.** `Harness::acceptance` is the static declaration.
  What only the running client says is learned by its codec and reported to
  the supervisor: OpenCode's `promptCapabilities.image` in its `initialize`
  answer, and whether the chosen model reads images where the client's model
  listing says so (`Model` gains its input kinds when the listing has them).
  The session's effective acceptance is the static declaration narrowed by
  what was learned; unknown is not yes (`Takes::resolve` with an `Offer`:
  offered, not offered, or unknown; built). The codecs' reports are phase
  3.
- `Limits`: declarative configuration (`[attachments]`), with defaults: 5 MB
  and 8000 px a side per image (the Anthropic API's limits), 10 attachments
  and 20 MB per prompt, and in phase 4, 25 MB per file. Inline delivery
  grows bytes by a third (base64). Built with its defaults (MiB):
  `Attachment::parse` checks an image's uploaded bytes and longest side,
  and `Limits::check_prompt` a prompt's count and its bytes in base64.
  `AttachmentOptions` is the `[attachments]` section, in bytes and pixels,
  read into `Limits` by the daemon's configuration. Built.
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
  session's folder. Built: the layout is the supervisor's pure
  `AttachmentFolders`, and `AgentHost::place_file`, `read_file`, and
  `remove_tree` do the disk work (`ProcessHost` in the daemon, which makes
  `<data>/attachments` at start; without it sessions take no
  attachments).
- **Scoped by session.** Deleting a session removes its folder (built, in
  `Supervisor::forget`, served or not); archiving keeps it; retention acts
  on a session's folder as a whole (not built: retention does not delete
  operator sessions yet). A client is
  given its own session's folder only (as an extra directory it may read,
  phase 3), never `attachments/`, so one session's agent cannot read
  another's screenshots.
- **Metadata** goes in a new `ren.sqlite3` table (`0012_attachments.sql`):
  id, operator session, name, image format, width, height, size, hash, and
  upload time, written from the session facts `Attached` and `Detached`
  (observation kinds `session_attached` and `session_detached`), deleted
  with the session, and read by `Reader::attachments`. Built. A run entry
  for a prompt (`run_entries`) names its attachments by id; image bytes
  never pass through the recorder queue, whose byte budget they would
  overrun.
- **The wire keeps a second copy.** An inline image is part of the provider
  request the gateway records. Capture policy and retention apply to it as
  to any body; the plan does not strip it.
- **Secrets.** `SecretScanner` cannot read pixels, and screenshots show
  tokens. Attachments are stored as uploaded, and the UI says so. Phone
  photos carry location in their EXIF data. **Decided:** Ren keeps and
  sends an image as uploaded, metadata included, and the composer says so
  (phase 3). Removing metadata means rewriting the image (orientation lives
  there too), and screenshots, the case to serve, carry none.

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

`attachment` in `ren-harness` is pure (the crate has no purity test; it
does I/O elsewhere). The supervisor's folder layout and each session's
attachments are pure in `attachments`, in its purity test; the writes go
through the `AgentHost` port, which the supervisor's tests fill in memory
and `TestGateway`'s on disk.

## API

- `POST /control/v1/sessions/{id}/attachments?name=…`: the raw bytes; a
  byte cap from `Limits`, enforced while reading. Answers the attachment's
  id and what it is, or a typed refusal (`docs/api.md`). Built.
- `GET /control/v1/sessions/{id}/attachments/{attachment}`: the bytes, for
  the UI's thumbnails. Non-cacheable, `Content-Type` from the parsed format,
  `Content-Disposition: inline` for images (`attachment` for files, phase
  4), and `nosniff`. Built.
- `DELETE` of one not yet sent in a prompt (a chip removed from the
  composer). One sent stays with the session. Built without the sent rule:
  nothing is sent until phase 3, which adds it.
- Listing a session's attachments: phase 3, with the composer that needs
  it.
- The prompt request (`PromptRequest`) gains `attachments: [AttachmentId]`.
- A session's view gains its effective `acceptance`, so the composer offers
  only what the client takes. The server checks again on send.
- The UI's `RunsProxy` (`ui/src/lib/server/control.ts`) forwards the upload
  and the read with the control token, like the other session routes.

## Clients

What each client takes, to confirm against its current documentation before
its codec is written (`AGENTS.md`: check official documentation before
changing client behavior):

| Client      | Image input                                                                        | Mid-turn steer with an image                     | Checked                                                 |
| ----------- | ---------------------------------------------------------------------------------- | ------------------------------------------------ | ------------------------------------------------------- |
| Codex       | `localImage` (a path) or `image` input items in `turn/start`                       | yes: `turn/steer` takes the same `UserInput`s    | 0.162.1, `codex app-server generate-json-schema`        |
| Claude Code | `image` content blocks (base64) in a stream-json user message                      | to confirm                                       | not yet: needs a real request                           |
| Pi          | `images` (`{ type: "image", data, mimeType }`) on the RPC `prompt` command         | yes: `images` on `steer` and `follow_up` too     | 1.5.2, its `rpc-types.d.ts`                             |
| OpenCode    | an ACP `image` content block, when `initialize` reports `promptCapabilities.image` | to confirm (its steering is a `_meta` extension) | 2.2.7 reports `image: true` and `additionalDirectories` |

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
   declaration. Unit tests (with a property test over arbitrary bytes),
   labels pinned. Built; `Prompt` moved to phase 3 (see Deviations).
2. **Storage and upload.** The migration, the session folder under
   `AgentHost`, the upload, read, and delete routes, deletion with the
   session, `[attachments]`, and the EXIF decision. Supervisor tests with an
   in-memory host, a store test, and a `TestGateway` test on disk across a
   restart. Built.
3. **Images through every client.** Each codec's encoding, the learned
   acceptance (OpenCode's capabilities, models' input kinds), `Prompt` with
   its attachments, refusals on
   send, run entries naming attachments, and the UI composer and
   conversation. Fake clients in `cargo xtask dev` take images, so the dev
   slot shows them. Not built.
4. **Files.** `AttachmentKind::File`, `MentionPath` delivery, each client
   given its session's folder to read, and the UI's file chips. Not built.
5. **Redline screenshots.** A Redline brief sends its marked screenshot as
   an image. Not built.

## Deviations

- **`Attachment` has no id.** `ren-harness` sits below `ren-recorder`, where
  Ren's identities live, so an `Attachment` is what was uploaded and the
  store pairs it with its `AttachmentId`.
- **`Prompt` is unchanged in phase 1.** What a prompt carries to a codec
  (an attachment's bytes, or its path) follows from delivery, which phase 3
  builds; a `Prompt` is stored and published as text, so changing its form
  before then would change a format for nothing that uses it.
- **`Limits` had defaults only in phase 1.** `[attachments]` came in phase
  2, with the upload that uses it.
- **The per-image byte limit is on the uploaded bytes.** Whether a provider
  counts an inline image's bytes before or after base64 is to confirm in
  phase 3, with each client; the prompt limit already counts base64.
- **`ByteSize` is the attachment module's.** The daemon's `ByteLimit` is a
  download cap in a crate above it.
- **The upload names its file in the query** (`?name=`), not a header:
  header values are ASCII, and file names are not.
- **A session's view does not carry its attachments.** `SessionView` is
  listed for every session; the supervisor reads a session's attachments
  apart (`Reader::attachments`) when it serves it from the store.
- **The table has image columns, not a kind.** Images are the only kind
  until phase 4, whose migration adds files.
- **A stored attachment Ren cannot read back is left out** of the session
  as served (a format this build does not know), with a warning; its row
  and file stay.
