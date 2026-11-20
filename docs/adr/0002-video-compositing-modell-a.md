# ADR 0002: Video-Compositing nach Modell A (DirectComposition, zwei Ebenen)

- Status: angenommen
- Datum: 2026-11-20
- Betroffene Crates: `lfx-mpv-sys`, `lfx-player`, `lfx-shell`

## Kontext
Gate G3: Die LGPL-libmpv (nur dynamisch geladen, DLL nie im Repo) soll Video mit HDR-Option hinter einer Slint-Oberfläche anzeigen. mpv 0.42 (Render-API 2.6) bietet für Windows den Render-Typ `d3d11`; die App stellt das `ID3D11Device` (Multithread-Schutz aktiv) und präsentiert selbst. Geprüft auf Win ARM64 (Adreno X1-85) mit `mpv-dev-lgpl-aarch64-20261119`.

## Entscheidung
Modell A: ein Fenster mit DirectComposition-Baum Wurzel -> Video-Visual (unten) -> UI-Visual (oben).
- Video: `CreateSwapChainForComposition`, FLIP_SEQUENTIAL, 2 Puffer; mpv rendert direkt in Backbuffer 0. Standard RGBA16F mit scRGB (`G10_NONE_P709`); für HDR-Quellen alternativ R10G10B10A2 mit PQ/BT.2020 (`G2084_NONE_P2020`).
- UI: eigene Swapchain mit vormultipliziertem Alpha (R8G8B8A8), gefüllt vom Slint-Software-Renderer, Slint in eigenem Thread.
- Billboard (Video im Rechteck der Startseite): Video-Swapchain verkleinern, Visual per Offset und Clip platzieren.
- Modell C ist unter Windows nicht nötig; für macOS (nur CI, kein Gerät) ist Modell C vorgesehen, dort ungeprüft.

## Alternativen
- Modell B (Slint-Overlay in der Video-Swapchain, ein Visual): nicht nötig, Modell A funktioniert.
- Modell C: unter Windows überflüssig, bleibt für macOS (`gpu-next` mit `--wid` gilt dort als unzuverlässig).
- libmpv2-Crate: nur OpenGL-Render-API, deshalb eigene FFI `lfx-mpv-sys`.

## Folgen
Messwerte (Spike, 60 s, 2560x1440 physisch, Ausgabe mit 60 Hz getaktet, 4K-Quelle 3840x2160@60):
- `mpv_render_context_create(d3d11)` = OK; RGBA16F und RGB10A2 funktionieren; `hwdec=d3d11va` (h264, HEVC 10 Bit) auf demselben Gerät.
- Leerlauf scRGB 59,5 fps, PQ 59,6 fps, Billboard mit scrollender UI 59,2 fps (UI 72 fps); Bildabstand p99 17,1-19,6 ms; Present p99 <= 2,9 ms; 0 Present-/Render-/Resize-Fehler, kein Schwarzbild in 27 Prüfungen. 10-12 Drops stammen aus dem Anlauf.
- Resize-Salven (Skript mit 21 Resizes, Maximieren, Minimieren): fehlerfrei, aber im Mittel 47 fps, p99-Bildabstand ca. 90 ms (UI-Neuzeichnen und Upload unter der Multithread-Sperre). Gegenmaßnahmen: nur Dirty-Rects hochladen, UI während Resize drosseln, Staging + Copy außerhalb der Render-Phase.
- HDR-Stand: Windows-HDR war aus. `CheckColorSpaceSupport`: scRGB Present+Overlay, PQ Present; `SetColorSpace1` und `SetHDRMetaData` laufen fehlerfrei. HDR10-Testclip: sig-peak 4,93, max-cll 1000, max-fall 400. `video-params/gamma` meldet für PQ-Quellen "bt.1886", daher HDR-Erkennung zusätzlich über sig-peak/Farbmatrix/`colorlevels`. Für scRGB `hdr-compute-peak=no` setzen (sonst 61 Drops, p50 22 ms).
- Offen: Sichttest mit eingeschaltetem Windows-HDR (scRGB-Weißpegel, `target-peak`/Gain, PQ und scRGB mit dem HDR10-Clip); Verhalten bei 90 Hz Ausgabe; x64 und macOS. Der Sichttest wird verschoben, bis HDR am Dev-Gerät eingeschaltet werden kann.
- `render_d3d11.h` liegt dem Paket nicht bei; die Strukturen sind nach dem Upstream-Header nachgebaut. `lfx-mpv-sys` bietet noch kein `get_property_node`/`command_node` (bei Bedarf für Spuren/Kapitel nachrüsten).
- Gate G3 gilt für Modell A als bestanden, mit offenem Sichttest für HDR.
