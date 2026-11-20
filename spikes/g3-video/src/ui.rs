//! Slint-OSD (Software-Renderer, Minimal-Plattform).

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PlatformError, WindowAdapter};

slint::slint! {
    #[edition = "1.19"]
    export component Osd inherits Window {
        background: transparent;
        in property <string> heading;
        in property <string> time-text;
        in property <string> info;
        in property <float> progress;
        in property <bool> paused;
        in property <bool> billboard;
        in property <length> scroll-x;
        callback toggle-pause();

        // Billboard-Seitenhintergrund (nur im Billboard-Modus): Video liegt als Rechteck darunter.
        if root.billboard : Rectangle {
            background: #101820e0;
            x: 0px; y: 0px; width: parent.width * 0.35; height: parent.height;
        }
        if root.billboard : Rectangle {
            background: #101820e0;
            x: 0px; y: parent.height * 0.53; width: parent.width; height: parent.height * 0.47;
        }
        if root.billboard : Rectangle {
            background: #101820e0;
            x: parent.width * 0.95; y: 0px; width: parent.width * 0.05; height: parent.height;
        }
        if root.billboard : Rectangle {
            background: #101820e0;
            x: 0px; y: 0px; width: parent.width; height: parent.height * 0.08;
        }
        // Scrollende Kachelzeile, ragt in das Video-Rechteck hinein.
        if root.billboard : Rectangle {
            y: parent.height * 0.48;
            height: parent.height * 0.40;
            clip: false;
            for i in 14 : Rectangle {
                x: i * 230px - root.scroll-x;
                y: 0px;
                width: 210px; height: 300px;
                border-radius: 10px;
                background: i == 0 ? #ff7a1a : (i == 1 ? #2f9e8f : (i == 2 ? #7a5cff : (i == 3 ? #d6336c : #3b4a5c)));
                Text {
                    text: "Film " + (i + 1001);
                    color: white;
                    font-size: 22px;
                    horizontal-alignment: center;
                    vertical-alignment: center;
                }
            }
        }

        // Titelzeile.
        Rectangle {
            x: 0px; y: 0px; width: parent.width; height: 64px;
            background: #000000a0;
            Text {
                x: 24px; width: parent.width - 48px; height: parent.height;
                text: root.heading;
                color: white;
                font-size: 22px;
                vertical-alignment: center;
            }
        }

        // Untere Leiste mit Fortschritt und Knopf.
        Rectangle {
            x: 0px; y: parent.height - 120px; width: parent.width; height: 120px;
            background: #000000b0;
            Rectangle {
                x: 24px; y: 22px; width: parent.width - 48px; height: 8px;
                border-radius: 4px;
                background: #ffffff40;
                Rectangle {
                    x: 0px; width: parent.width * root.progress; height: parent.height;
                    border-radius: 4px;
                    background: #ff7a1a;
                }
            }
            Text {
                x: 24px; y: 40px; width: 300px; height: 40px;
                text: root.time-text;
                color: white;
                font-size: 20px;
                vertical-alignment: center;
            }
            Text {
                x: 340px; y: 40px; width: parent.width - 700px; height: 40px;
                text: root.info;
                color: #c8d0d8;
                font-size: 16px;
                vertical-alignment: center;
                overflow: elide;
            }
            Rectangle {
                x: parent.width - 220px; y: 36px; width: 196px; height: 48px;
                border-radius: 24px;
                background: ta.has-hover ? #ffffff50 : #ffffff30;
                Text { text: root.paused ? "Weiter" : "Pause"; color: white; font-size: 20px; }
                ta := TouchArea { clicked => { root.toggle-pause(); } }
            }
        }
    }
}

pub struct SpikePlatform {
    pub window: Rc<MinimalSoftwareWindow>,
    start: Instant,
}

impl Platform for SpikePlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
}

/// Setzt die Plattform und liefert Fenster + Komponente (nur auf dem gfx-Thread aufrufen).
pub fn create() -> (Rc<MinimalSoftwareWindow>, Osd) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(SpikePlatform {
        window: window.clone(),
        start: Instant::now(),
    }))
    .expect("Slint-Plattform");
    let osd = Osd::new().expect("Osd");
    (window, osd)
}
