# Architecture

## Ownership

`http-router-only` is an inference router, not a model supervisor. Each Python backend owns eager model loading, explicit operator load/unload routes, and cleanup on `Ctrl+C`. The gateway only observes health and submits inference.

```text
client / ER SDK
      |
      v
  vox-http :8180
   |       |       |       |
   v       v       v       v
Crisper Translate LongCat Router
 :8172    :8176   :8230   :8182
  STT   translate  TTS   audio I/O
```

## Supported compositions

1. STT -> text
2. STT -> translate -> text
3. text -> translate -> text
4. STT -> TTS
5. STT -> translate -> TTS
6. text -> translate -> TTS
7. text -> TTS
8. file -> configured microphone router (VB-CABLE on Windows) or default playback
9. microphone or system audio -> STT -> optional translate -> text
10. microphone or system audio -> STT -> optional translate -> LongCat -> playback/mic/WAV

The streaming contract separates inference cadence from client paint cadence.
Each `/v1/captions/{id}` background worker owns a uniquely named,
non-destructive router cursor and atomically publishes complete snapshots.
Workers never replace one another unless the same ID is restarted. GET
polling is an in-memory read: the prior text remains available with
`processing=true` until a new revision is complete.

Translation is not a separate side channel. The same translation primitive is
composed into `/v1/transcribe`, `/v1/speak`, and
`/v1/transcribe-and-speak`, while `/v1/translate` exposes it directly. Named
`inbound`/`outbound` routes supply target defaults. Every translating request
may override `target_language` for that call. `source_language` remains in the
wire schema for compatibility, but multilingual EraX does not use it as an
inference gate; spoken-language tokens belong only to CrisperWhisper.

Desktop Vox and this HTTP member both call `crates/vox-local-core` for CrisperWhisper and LongCat. LongCat uses a configurable character target only to locate a complete sentence boundary: it scans backward first, forward only when necessary, and never splits an unpunctuated sentence. If the backend rejects a multi-sentence request, the core retreats one additional sentence and retries. Successful WAVs are merged into one valid container. Disabling concatenation bypasses all splitting and retry behavior.

The HTTP-specific crate owns only Axum state and routes, private-network URL validation, translation proxying, a map of named cached-caption workers, and Vox router proxying. Desktop tray, settings, hotkeys, clipboard, and device concerns never enter its dependency graph.

Named voice pairs use server-local `.wav`, `.mp3`, or `.m4a` reference audio plus a verbatim UTF-8 `.txt` transcript. Configuration validates that contract at startup. TTS routes accept an optional per-request `seed`; responses echo the effective seed in `X-Vox-Seed`. This preserves Vox's selectable-pair and seed behavior without introducing mutable tray-style state into an HTTP gateway.

## Security boundary

- Runtime state is immutable apart from the reusable HTTP client and explicitly requested named caption snapshots. Ordinary request text, audio, routes, voices, and seeds are never retained after a response. The router alone owns live audio queues and fans loopback into independent desktop/API consumer lanes.
- The only automatic disk write is first-run creation of `config.toml` beside the executable (or at an explicit `VOX_HTTP_CONFIG` path).
- Backend URLs are rejected unless they are loopback, private IPv4, IPv6 ULA/link-local, or `.local`.
- Optional bearer authentication protects inference routes.
- `GET /v1/routes` exposes voice names, never reference paths or transcripts.
- There are no gateway routes for `/load`, `/unload`, process execution, configuration mutation, or arbitrary URL fetches. Audio routes call only the configured private Vox router.
- CORS is explicit and configurable for local browser/Even Realities clients.
