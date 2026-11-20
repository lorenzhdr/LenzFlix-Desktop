# ADR 0001: UI mit Slint, Renderer femtovg-wgpu

- Status: angenommen
- Datum: 2026-11-20
- Betroffene Crates: `lfx-ui`, `lfx-shell`

## Kontext
Die App ist voll nativ in Rust (Win ARM64, Win x64, macOS Apple Silicon). Gate G2 verlangt, dass eine Startseiten-Attrappe (10 Zeilen x 20 synthetische Cover, horizontales und vertikales Scrollen, Hover-/Fokus-Zoom mit Schatten) auf dem Dev-Laptop (Snapdragon X Elite, Adreno X1-85, 2944x1840 px, DPR 2, Anzeige 89 Hz, Netzteil) mindestens 85 fps bei p95 <= 11,7 ms erreicht. Fallback bei Fehlschlag wäre iced.

## Entscheidung
Slint 1.19 (Royalty-free-Lizenz) mit dem Renderer `femtovg-wgpu` (`SLINT_BACKEND=winit-femtovg-wgpu`) als Erstwahl. Skia (D3D) bleibt als optionales Feature. Eigene Titelleiste (Fenster ohne Rahmen); die Snap-Layouts von Windows 11 werden später in `lfx-shell` per `WM_NCHITTEST` (`HTMAXBUTTON`, Window-Subclass über das HWND) ergänzt.

## Alternativen
- Skia (D3D): beste Werte bei p99/max, aber der vorgebaute Skia-Build lässt sich mit den installierten MSVC-Build-Tools (14.44) auf Windows ARM64 nicht linken (12 fehlende STL-Symbole, im Spike nur per skalarem Shim umgangen). Nur Option, bis ein neueres MSVC-Toolset verfügbar ist.
- femtovg (OpenGL, Standard): 74,8 fps, p95 22,2 ms; verpasste VSyncs im GL-Pfad dieser GPU, unabhängig von Schatten, Scrollart oder Fokus. Verfehlt G2.
- skia-opengl (81,0 fps) und Software (76,8 fps): verfehlen G2; Software höchstens als Rückfall ohne GPU.
- iced: wegen bestandenem G2 nicht nötig.

## Folgen
Messwerte (15 s, 3 Läufe je Renderer, JSON in den Spike-Berichten):

| Renderer | fps | p95 | p99 | max | Frames > 20 ms | G2 |
|---|---|---|---|---|---|---|
| Skia (D3D) | 87,3-88,4 | 11,56-11,62 ms | 13,2-18,4 ms | 20,8-52,8 ms | 1-7 | ja |
| femtovg-wgpu | 86,6-87,2 | 11,57-11,61 ms | 14,0-14,3 ms | 50-67 ms | 9-11 | ja |
| femtovg (GL) | 74,8 | 22,2 ms | 31,6 ms | 42 ms | 83 | nein |
| skia-opengl | 81,0 | 22,2 ms | 22,4 ms | 33 ms | 121 | nein |
| Software | 76,8 | 23,3 ms | 28,8 ms | 32 ms | 86 | nein |

- femtovg-wgpu liegt beim p95 innerhalb von 1 % von Skia, hat etwas mehr Ausreißer, braucht aber weder Skia-Download noch STL-Shims und passt voraussichtlich besser zur D3D-Anbindung des Videos (siehe ADR 0002).
- Die Schwelle ist bei 89 Hz knapp (Intervall 11,24 ms); gemessen wurde der Abstand der `RedrawRequested`-Ereignisse, nicht die GPU-Zeit.
- Variable Schriftgewichte (Inter 400-800) funktionieren in Slint 1.19.
- Nicht belegt: interaktives Ziehen, Doppelklick, Rand-Resize, Mausrad, Aero-Snap, Snap-Layouts (nur Code), zweiter Monitor, HDR, macOS (Metal-Skia bzw. femtovg-wgpu dort unbewertet).
- Folgearbeit: Snap-Layouts in `lfx-shell` umsetzen (Fallback ohne Flyout akzeptabel); Renderer-Wahl per Feature, nach G3 endgültig festlegen; AboutSlint in App und README.
