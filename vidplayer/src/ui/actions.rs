use std::time::Duration;

use gpui::{App, KeyBinding};

use super::app_state::AppState;

gpui::actions!(
    vidplayer,
    [
        TogglePause,
        ToggleMute,
        VolumeUp,
        VolumeDown,
        SeekForward,
        SeekBackward,
        SeekForwardLarge,
        SeekBackwardLarge,
        Quit,
    ]
);

const SEEK_SMALL: Duration = Duration::from_secs(10);
const SEEK_LARGE: Duration = Duration::from_secs(30);

pub fn register_shortcuts(app: &mut App) {
    app.bind_keys(key_bindings());

    app.on_action(|_: &TogglePause, app: &mut App| {
        let state = app.global_mut::<AppState>();
        if let Some(ref player) = state.player {
            player.toggle_pause();
            let paused = player.is_paused();
            println!("Playback {}", if paused { "paused" } else { "resumed" });
        }
    });

    app.on_action(|_: &ToggleMute, app: &mut App| {
        let state = app.global_mut::<AppState>();
        if let Some(ref player) = state.player {
            let muted = player.toggle_mute();
            println!("Audio {}", if muted { "muted" } else { "unmuted" });
        }
    });

    app.on_action(|_: &VolumeUp, app: &mut App| {
        let state = app.global_mut::<AppState>();
        state.adjust_volume(0.1);
        println!("Volume: {:.0}%", state.volume * 100.0);
    });

    app.on_action(|_: &VolumeDown, app: &mut App| {
        let state = app.global_mut::<AppState>();
        state.adjust_volume(-0.1);
        println!("Volume: {:.0}%", state.volume * 100.0);
    });

    app.on_action(|_: &SeekForward, app: &mut App| {
        let state = app.global::<AppState>();
        if let Some(ref player) = state.player {
            player.seek_forward(SEEK_SMALL);
            println!("Seeked forward 10s");
        }
    });

    app.on_action(|_: &SeekBackward, app: &mut App| {
        let state = app.global::<AppState>();
        if let Some(ref player) = state.player {
            player.seek_backward(SEEK_SMALL);
            println!("Seeked backward 10s");
        }
    });

    app.on_action(|_: &SeekForwardLarge, app: &mut App| {
        let state = app.global::<AppState>();
        if let Some(ref player) = state.player {
            player.seek_forward(SEEK_LARGE);
            println!("Seeked forward 30s");
        }
    });

    app.on_action(|_: &SeekBackwardLarge, app: &mut App| {
        let state = app.global::<AppState>();
        if let Some(ref player) = state.player {
            player.seek_backward(SEEK_LARGE);
            println!("Seeked backward 30s");
        }
    });

    app.on_action(|_: &Quit, app: &mut App| {
        println!("Quitting...");
        app.quit();
    });
}

fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("space", TogglePause, None),
        KeyBinding::new("m", ToggleMute, None),
        KeyBinding::new("up", VolumeUp, None),
        KeyBinding::new("down", VolumeDown, None),
        KeyBinding::new("right", SeekForward, None),
        KeyBinding::new("left", SeekBackward, None),
        KeyBinding::new("shift-right", SeekForwardLarge, None),
        KeyBinding::new("shift-left", SeekBackwardLarge, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]
}
