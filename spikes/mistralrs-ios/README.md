# mistral.rs on the iPad: spike

Whether mistral.rs could run Alcyo's local board model on the iPad instead of llama.cpp (#41).

- It builds for `aarch64-apple-ios` with Metal (CI: `.github/workflows/spike-mistralrs.yml`), with sysinfo patched in
  `vendor/` (libc declares two Darwin items for the Mac only).
- `replay.jsonl`: 459 board requests recorded from Alcyo's eval cases against llama-server with Qwen3-4B Instruct
  2507 Q4_K_M, each with its grammar converted to Lark by llguidance (`prepare.py`) and llama-server's answer.
- On an Apple-silicon Mac: `sh compare.sh <mistralrs-ios-spike>` times llama-server and mistral.rs on all of them
  with the same model, and says how often each gives llama-server's recorded answer.
