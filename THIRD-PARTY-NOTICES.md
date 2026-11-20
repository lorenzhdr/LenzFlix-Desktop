# Drittanbieter-Hinweise

LENZFLIX Desktop selbst steht unter der MIT-Lizenz (siehe `LICENSE`). Die folgenden Komponenten stammen von Dritten.

## libmpv und FFmpeg (LGPL-2.1 oder neuer)

Die App spielt Medien über libmpv, das FFmpeg-Bibliotheken enthält. Verwendet werden ausschließlich LGPL-Builds (ohne GPL-Teile). Die Bibliothek wird zur Laufzeit dynamisch geladen und liegt der App als eigene Datei bei; sie kann durch eine eigene, kompatible Fassung ersetzt werden. Die App enthält keinen Code aus libmpv.

- mpv: https://mpv.io, Quelltext: https://github.com/mpv-player/mpv
- FFmpeg: https://ffmpeg.org, Quelltext: https://git.ffmpeg.org/ffmpeg.git
- Windows-Builds: https://github.com/zhongfly/mpv-winbuild (genaue Version und Prüfsumme in `xtask/mpv-pins.toml`)
- macOS-Builds: https://github.com/media-kit/libmpv-darwin-build

Die Lizenztexte liegen den Archiven bei und werden mit der App ausgeliefert.

## Slint (Royalty-free 2.0)

Die Oberfläche nutzt Slint (https://slint.dev) unter der Lizenz „Slint Royalty-free 2.0“. Die App zeigt dazu den vorgeschriebenen Hinweis „Made with Slint“.

## Schriften (SIL Open Font License 1.1)

- Inter
- Figtree
- Source Sans 3
- Space Mono
- VT323

Die Schriften stehen unter der SIL Open Font License 1.1 (https://openfontlicense.org).

## Rust-Bibliotheken

Weitere Rust-Crates stehen unter permissiven Lizenzen (MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0, Unicode-3.0). Die Prüfung erfolgt per `cargo deny` (siehe `deny.toml`).
