use gpui::{App, KeyBinding};

use super::app_state::AppState;

gpui::actions!(
    vidwall,
    [
        TogglePause, // Space - pause/resume all videos
        ToggleMute,  // M - mute/unmute all videos
        VolumeUp,    // Up arrow - increase master volume
        VolumeDown,  // Down arrow - decrease master volume
        SkipAll,     // Enter - skip all videos and load new ones
        Quit,        // Cmd+Q - quit the application
    ]
);

/**
    Register all keyboard shortcuts and their handlers at the app level.
*/
pub fn register_shortcuts(app: &mut App) {
    // Bind keys to actions
    app.bind_keys(key_bindings());

    // Register action handlers
    app.on_action(|_: &TogglePause, app: &mut App| {
        let state = app.global_mut::<AppState>();
        let paused = state.toggle_pause();
        println!("Playback {}", if paused { "paused" } else { "resumed" });
    });

    app.on_action(|_: &ToggleMute, app: &mut App| {
        let state = app.global_mut::<AppState>();
        let muted = state.toggle_mute();
        println!("Audio {}", if muted { "muted" } else { "unmuted" });
    });

    app.on_action(|_: &VolumeUp, app: &mut App| {
        let state = app.global_mut::<AppState>();
        state.adjust_volume(0.1);
        println!("Volume: {:.0}%", state.master_volume * 100.0);
    });

    app.on_action(|_: &VolumeDown, app: &mut App| {
        let state = app.global_mut::<AppState>();
        state.adjust_volume(-0.1);
        println!("Volume: {:.0}%", state.master_volume * 100.0);
    });

    app.on_action(|_: &SkipAll, app: &mut App| {
        let state = app.global_mut::<AppState>();
        state.request_skip_all();
        println!("Skipping all videos...");
    });

    app.on_action(|_: &Quit, app: &mut App| {
        println!("Quitting...");
        app.quit();
    });
}

/**
    Define key bindings for all actions.
*/
fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("space", TogglePause, None),
        KeyBinding::new("m", ToggleMute, None),
        KeyBinding::new("up", VolumeUp, None),
        KeyBinding::new("down", VolumeDown, None),
        KeyBinding::new("enter", SkipAll, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]
}
