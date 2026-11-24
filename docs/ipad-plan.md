# The iPad plan

iPad is Alcyo's main target. No iPad is available for testing yet, so nothing here has been run on one. Each
claim below is marked as checked in the source, or as a question for a real device.

## What already carries over

| Part | State | How we know |
| --- | --- | --- |
| Board, meetings, marks, layout | Plain Rust with no platform calls | Their crates' code and tests |
| Window and rendering | GPUI has an iOS platform layer, `gpui-pre-ios`, with touch events | Source only |
| Styled components | `gpui-component` is the same on every platform | Source only |
| Speech recognition | `sherpa-onnx`'s build script has an iOS path | Source only |
| Microphone | `cpal` mentions iOS | Source only; it may need an audio session set up |
| Local model | llama.cpp's server runs inside the app (option B below) | Started, asked and stopped on a CI Mac; not run on an iPad |

## The local model

Today the app starts a prebuilt `llama-server` as a child process and talks to it over local HTTP. That has to change on
iPad, and only there.

| Option | How | For | Against |
| --- | --- | --- | --- |
| A. Child process (today) | A separate process and HTTP | A crash leaves the app and the meeting alive, and it can fall back to the CPU; upgrading swaps files | Not allowed on iPad |
| B. Server in the app | Build llama.cpp's server as a library and run `llama_server()` on a thread, still over local HTTP | The code above it barely changes, and the prompt cache, templates, grammar and vision come along | A crash or `exit` takes the app with it; it must be built from source |
| C. Direct calls | `llama-cpp-2` bindings to libllama | No port, real load-progress callbacks | The chat template, `--fit`, the prompt cache and vision are server features we would rewrite |
| D. MLX | Apple's own framework | May be faster on Apple silicon | See below |

**Recommendation:** keep A on the desktop, because surviving a model crash matters during a meeting. Try B on iPad first,
and fall back to C if it won't run there. The first step, which helps either way, is to put an engine interface in
`alcyo-agent` so that `Backend::ManagedLocal` doesn't assume a child process. It would cover starting, reporting
progress, asking and stopping.

**Why not MLX first:** the board relies on GBNF grammar to force legal operations, and as far as we know MLX has no ready
equivalent. It would also mean a second model format and re-running every evaluation, while Linux would still need
llama.cpp. Compare it on a real device once the engine interface exists, and add it as another engine if it is clearly
faster or lighter. Today's first line takes about 0.5 s on an RTX 3060 Laptop, so reliable output matters more right now.

**Done: B.** A mistral.rs spike (branch `spike/mistralrs-ios`) refused the default model on a CI Mac, so the iPad runs
llama.cpp's own server inside the app:

- **Library.** Our fork, HsiangNianian/llama.cpp, tag `alcyo-b11954-2` (branch `alcyo/b11954`, upstream b11954 as the
  desktop uses), adds `llama_server_main(argc, argv)`, which installs no signal handlers and never exits the process, and
  `llama_server_shutdown()`, which may be called from any thread at any time and also cancels a model still loading.
  `scripts/build-llama-embedded.sh` builds it as static libraries with Metal and its shaders built in, once per tag;
  the archives sit on the fork's release, and `.cargo/llama-embedded.sha256` pins them as sherpa-onnx's are.
- **In the app.** `crates/llama` links them and runs the server on a thread with a 16 MiB stack; dropping it stops it.
  `alcyo-agent` starts it with the desktop's arguments on the same port and talks to it over the same HTTP, so only
  `start_llama` differs. The desktop still starts a child process.
- **Memory.** 6144 tokens of context as on the desktop, keys and values at 8 bits (`-ctk q8_0 -ctv q8_0 -fa on`, about
  0.45 GB less for Qwen3-4B), `--cache-ram 256`, `--fit on`. Before starting, the model's size plus 1.25 GB is compared
  with `os_proc_available_memory()`, and if it does not fit the app says so rather than being ended by iPadOS. The
  option is offered on iPads with 8 GB or more. The app asks for `com.apple.developer.kernel.increased-memory-limit`,
  which its provisioning profile has to grant.
- **Models** are downloaded as on the desktop, into Application Support, kept out of backups.

## What to measure on a real device

1. **Does the model fit?** Qwen 4B took about 3 GB of graphics memory here. iPadOS limits how much an app may use, and the
   limit depends on the model of iPad. This decides whether the default stays 4B or falls to MiniCPM5-2B, which took about
   1.8 GB with lower quality.
2. Load time, first line and later lines, under Metal.
3. Heat and battery over an hour-long meeting.
4. Pencil latency on the board.
5. Whether option B survives the app going to the background. iPadOS may suspend the thread or end a local listener.
6. Whether a large model file can be downloaded in the background.

## What the interface needs

These can be done before any device arrives.

- **Touch targets:** icon buttons rely on tooltips, which a touch screen doesn't have, so the main actions need labels.
- **Hover:** tracing a speaker already works by choosing them in the panel, and nothing else may depend on hover.
- **Pencil and finger:** the board needs to tell them apart, for example the pencil marks and the finger pans. Today's
  marks use a click and a toolbar.
- **Panels:** a 340 px side panel over a board needs checking in portrait.
- **Files:** Unix-only code remains in `alcyo-models` and `alcyo-agent`, and imports and exports assume paths. These
  need iPadOS's file picker and share sheet.
- **Updates:** the App Store handles them, so the in-app updater stays out.
- **Remote model:** it matters more here, as an iPad that can't fit the local model can still draw boards.

## Order

1. Mac first, as it shakes out Apple builds and Metal, and it can be tested on a borrowed M5 Air.
2. The engine interface in `alcyo-agent`, with no change in behavior.
3. The interface items above, testable on a desktop window.
4. A build for the iPad simulator in CI: first with the remote model only, then option B. A simulator can't answer the
   questions about memory, heat or the pencil.
5. A real iPad, starting with the measurements above.
