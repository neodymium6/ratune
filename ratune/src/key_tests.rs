use super::*;

#[test]
fn jukebox_switch_is_available_on_every_tab_and_clears_pending_gg() {
    let kb = Keybinds::from_section(&config::KeybindsSection::default());
    for tab in [Tab::Home, Tab::Browser, Tab::NowPlaying] {
        let mut pending = true;
        assert!(matches!(
            map_key(
                KeyCode::F(8),
                KeyModifiers::NONE,
                tab,
                &kb,
                &mut pending,
                false
            ),
            Action::ToggleJukebox
        ));
        assert!(!pending);
    }
}

#[test]
fn default_mix_key_is_available_on_all_tabs() {
    let kb = Keybinds::from_section(&config::KeybindsSection::default());
    for tab in [Tab::Browser, Tab::NowPlaying, Tab::Home] {
        assert!(matches!(
            map_key(
                KeyCode::Char('m'),
                KeyModifiers::NONE,
                tab,
                &kb,
                &mut false,
                false
            ),
            Action::InstantMix
        ));
    }
}

#[test]
fn mix_key_can_be_rebound_or_disabled() {
    let sec = config::KeybindsSection {
        instant_mix: Some("Shift+m".into()),
        ..Default::default()
    };
    let kb = Keybinds::from_section(&sec);
    assert!(matches!(
        map_key(
            KeyCode::Char('m'),
            KeyModifiers::SHIFT,
            Tab::Browser,
            &kb,
            &mut false,
            false
        ),
        Action::InstantMix
    ));
    assert!(!matches!(
        map_key(
            KeyCode::Char('m'),
            KeyModifiers::NONE,
            Tab::Browser,
            &kb,
            &mut false,
            false
        ),
        Action::InstantMix
    ));
    let kb = Keybinds::from_section(&config::KeybindsSection {
        instant_mix: Some(String::new()),
        ..Default::default()
    });
    assert!(kb.instant_mix.is_none());
    assert!(!matches!(
        map_key(
            KeyCode::Char('m'),
            KeyModifiers::NONE,
            Tab::Browser,
            &kb,
            &mut false,
            false
        ),
        Action::InstantMix
    ));
}
