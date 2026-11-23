# Alcyo for iPad: design

The iPad layout, as mockups to build from (#41's UI). Open `index.html` in a browser; each page is one screen at
true point size (11" 1194 × 834, 13" 1376 × 1032, portrait 834 × 1194) with the design notes beside it, and
`shots/` holds a picture of each. Colours are the app's own, from `crates/ui/src/theme.rs`.

Direction A, canvas first, is the one to build; B (split view) and C (presenter) are kept for comparison, and C may
later become the mode for an external display.

Fonts: DM Sans and JetBrains Mono (SIL OFL 1.1, `fonts/OFL-*.txt`) and the Latin part of Excalifont (SIL OFL 1.1,
`fonts/OFL-Excalifont.txt`), as the app uses them. Chinese falls back to the system's Noto Sans CJK.
