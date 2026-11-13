//! The tray icon (B37): a StatusNotifierItem over D-Bus, pure Rust via `ksni`.
//!
//! The menu is the wallpaper's remote control (B48a: and the city's voice):
//! the city's name as the title, the headcount and era, the latest story lines
//! newest first, then the controls — pause the animation (the sim never
//! pauses — D15), a 60-second time-lapse, the weather right now, open the
//! settings window, start a new city (the old save is kept as a backup) and
//! quit. The display lines come from a shared [`Display`] the render loop
//! refreshes; each control item sends a [`TrayAction`] down a channel to the
//! render loop, so the tray thread never touches the scene.

use std::sync::{mpsc::Sender, Arc, RwLock};

use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem};
use ksni::{Icon, Tray};

use present::TrayAction;
use sim::story::TrayStatus;

/// B48a: the city's picture, shared between the render loop (which refreshes
/// it a few times a minute) and the tray thread (which reads it when the menu
/// opens, or the title is requested). The lock is held only for the copy.
pub type Display = Arc<RwLock<TrayStatus>>;

/// The tray item, holding the channel to the render loop.
pub struct CityTray {
    pub tx: Sender<TrayAction>,
    /// "Pause animation" is a checkbox in the menu; the wallpaper flips it.
    pub paused: bool,
    /// A one-line status shown as the tooltip description.
    pub status: String,
    /// The city's name and the latest story lines (B48a), or none when the
    /// wallpaper runs without a shared display.
    pub display: Option<Display>,
}

impl CityTray {
    /// The city's name right now (the tray's title, D68).
    fn city_name(&self) -> Option<String> {
        self.display
            .as_ref()
            .and_then(|d| d.read().ok())
            .filter(|s| !s.city.trim().is_empty())
            .map(|s| s.city.clone())
    }

    /// The headcount + era line, when the display holds one.
    fn head_line(&self) -> Option<String> {
        self.display
            .as_ref()
            .and_then(|d| d.read().ok())
            .and_then(|s| (!s.line.trim().is_empty()).then_some(s.line.clone()))
    }

    /// The latest feed lines, newest first (the "just watch" toggle gives none).
    fn feed(&self) -> Vec<String> {
        self.display
            .as_ref()
            .and_then(|d| d.read().ok())
            .map(|s| s.feed.clone())
            .unwrap_or_default()
    }
}

impl Tray for CityTray {
    fn id(&self) -> String {
        "citylive-iso".to_string()
    }

    /// The city answers to its own name (D68); a world that has not named
    /// itself yet (fresh start, save from before the naming) keeps the app's.
    fn title(&self) -> String {
        self.city_name().unwrap_or_else(|| "CityLive Iso".to_string())
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let city = self.city_name().unwrap_or_else(|| "CityLive Iso".to_string());
        let mut description = self.status.clone();
        if let Some(line) = self.head_line() {
            if description.is_empty() {
                description = line;
            } else {
                description = format!("{description} — {line}");
            }
        }
        ksni::ToolTip {
            title: city,
            description,
            ..Default::default()
        }
    }

    /// A 16×16 icon: a small iso diamond of roofs, drawn in code.
    fn icon_pixmap(&self) -> Vec<Icon> {
        const W: usize = 16;
        let mut data = vec![0u8; W * W * 4];
        let put = |data: &mut Vec<u8>, x: usize, y: usize, c: [u8; 4]| {
            if x < W && y < W {
                let i = (y * W + x) * 4;
                data[i..i + 4].copy_from_slice(&c);
            }
        };
        // Sky.
        for y in 0..W {
            for x in 0..W {
                put(&mut data, x, y, [0x1b, 0x26, 0x3c, 0xff]);
            }
        }
        // Two roofs and a tower, warm windows.
        for (cx, top, half) in [(5usize, 6usize, 4usize), (11, 8, 3)] {
            for dy in 0..4 {
                let w = half * (dy + 1) / 4;
                for dx in 0..=w {
                    let c = if dy == 0 {
                        [0xd8, 0x6a, 0x4a, 0xff]
                    } else {
                        [0x8a, 0x4a, 0x3a, 0xff]
                    };
                    put(&mut data, cx + dx, top + dy, c);
                    if dx > 0 {
                        put(&mut data, cx - dx, top + dy, c);
                    }
                }
            }
            for y in top + 4..14 {
                for x in cx.saturating_sub(3)..=cx + 3 {
                    put(&mut data, x, y, [0x3a, 0x44, 0x58, 0xff]);
                }
            }
        }
        for y in 4..14 {
            for x in 7..9 {
                put(&mut data, x, y, [0x5a, 0x6a, 0x86, 0xff]);
            }
        }
        for y in 6..12 {
            put(&mut data, 7, y, [0xff, 0xd2, 0x7e, 0xff]);
        }
        put(&mut data, 8, 7, [0xff, 0xd2, 0x7e, 0xff]);
        vec![Icon {
            width: W as i32,
            height: W as i32,
            data,
        }]
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let send = |tx: &Sender<TrayAction>, a: TrayAction| {
            let tx = tx.clone();
            move |_: &mut Self| {
                let _ = tx.send(a);
            }
        };
        // B48a: the city's voice — the name is the title, then the headcount
        // line and the latest story lines newest first (a disabled line each;
        // the tray has no label item, and these are to be read, not pressed).
        let mut menu: Vec<MenuItem<Self>> = Vec::new();
        if let Some(line) = self.head_line() {
            menu.push(
                StandardItem {
                    label: line.clone(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        }
        for l in self.feed() {
            menu.push(
                StandardItem {
                    label: l,
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        }
        if !menu.is_empty() {
            menu.push(MenuItem::Separator);
        }
        menu.extend([
            CheckmarkItem {
                label: "Pause animation".into(),
                checked: self.paused,
                activate: Box::new(send(&self.tx, TrayAction::TogglePause)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Time-lapse for 60 s".into(),
                activate: Box::new(send(&self.tx, TrayAction::Timelapse)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Weather now".into(),
                activate: Box::new(send(&self.tx, TrayAction::WeatherNow)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Settings…".into(),
                activate: Box::new(send(&self.tx, TrayAction::OpenSettings)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Open chronicle".into(),
                activate: Box::new(send(&self.tx, TrayAction::OpenChronicle)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Open worlds folder".into(),
                activate: Box::new(send(&self.tx, TrayAction::OpenWorlds)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Start a new city…".into(),
                activate: Box::new(send(&self.tx, TrayAction::NewCity)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(send(&self.tx, TrayAction::Quit)),
                ..Default::default()
            }
            .into(),
        ]);
        menu
    }
}

/// Start the tray on its own thread. Returns the receiver for the render loop;
/// a failure (no StatusNotifierWatcher on this desktop) is reported and the
/// wallpaper carries on without a tray.
pub fn spawn(status: String, display: Option<Display>) -> Option<std::sync::mpsc::Receiver<TrayAction>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let tray = CityTray {
        tx,
        paused: false,
        status,
        display,
    };
    match std::thread::Builder::new()
        .name("citylive-tray".into())
        .spawn(move || {
            // `spawn` owns the tray and runs the D-Bus service on this thread's
            // runtime; the handle is dropped, the service lives on.
            if let Err(e) = tray.spawn() {
                eprintln!("tray: {e} (no StatusNotifierWatcher?)");
            }
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }) {
        Ok(_) => Some(rx),
        Err(e) => {
            eprintln!("tray: cannot start thread: {e}");
            None
        }
    }
}
