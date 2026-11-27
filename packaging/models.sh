# Shared by the package scripts. Since 0.35 there is one package per platform: the speech models are voice packs,
# downloaded by the app (crates/models/src/voice), so a package carries only the voice detector, besides the meeting
# search model. The variants of 0.34 and before (full, -en, -ja, -none) are no longer made; the update manifest
# sends their installs to the one package (crates/tools/src/release.rs), and the app downloads the packs they came
# with on its first start.

# copy_voice <voice dir in the package>: what every package carries for speech, the voice detector.
copy_voice() {
  mkdir -p "$1"
  cp resources/voice/silero_vad.onnx "$1/"
}

# copy_search <search dir in the package>: the meeting search model, which every package carries, with the
# notice the Gemma Terms of Use ask to go with it (the terms themselves are among the licenses).
copy_search() {
  mkdir -p "$1"
  cp resources/search/*.gguf "$1/"
  cp assets/licenses/gemma-NOTICE.txt "$1/NOTICE.txt"
}
