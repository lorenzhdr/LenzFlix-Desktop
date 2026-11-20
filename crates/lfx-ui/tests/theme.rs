//! Theme-Umschaltung im Slint-Testing-Backend.

use lfx_ui::{Design, Theme, ThemeProbe};
use slint::{Color, ComponentHandle};

#[test]
fn theme_switches_tokens() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = ThemeProbe::new().expect("Komponente laden");
    let theme = ui.global::<Theme>();

    assert_eq!(theme.get_design(), Design::Flix);
    let flix = theme.get_t();
    assert_eq!(flix.red, Color::from_rgb_u8(0xe5, 0x09, 0x14));
    assert_eq!(flix.font_family.as_str(), "Inter");

    theme.set_design(Design::Retro);
    let retro = theme.get_t();
    assert_ne!(retro.red, flix.red);
    assert_ne!(retro.bg, flix.bg);

    theme.set_design(Design::Plus);
    let plus = theme.get_t();
    assert_ne!(plus.shadow_card.y, flix.shadow_card.y);

    theme.set_design(Design::Prime);
    assert_ne!(theme.get_t().bg, retro.bg);

    theme.set_design(Design::Flix);
    assert_eq!(theme.get_t().red, flix.red);
}
