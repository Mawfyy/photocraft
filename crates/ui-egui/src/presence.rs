//! What PhotoCraft is doing, for an outside status service (Discord Rich Presence, #4148).
//!
//! The shell (the native app) owns the service; this crate only reports what changed and never
//! knows what Discord is. The native app installs a hook in [`crate::Services::presence`]; the
//! web build and the tests leave it unset, and nothing here runs on the web.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::PhotocraftApp;

/// What the user is doing, as an outside service shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PresenceInfo {
    /// The open document's file name, without any directory part. Empty when the user keeps the
    /// name private (the default) or when nothing is open.
    pub document: String,
    /// A document is open, whatever the name is worth showing.
    pub editing: bool,
    /// The active tool, without the "Tool" suffix ("Brush").
    pub tool: String,
    /// The document's pixel size, when one is open.
    pub size: Option<(u32, u32)>,
    /// The canvas zoom, 1.0 = 100 %.
    pub zoom: f32,
    /// The document has unsaved changes.
    pub dirty: bool,
}

/// The shell's hook: called with the new state when it changes.
pub type Hook = Arc<dyn Fn(PresenceInfo) + Send + Sync>;

/// How long to wait between two reports. A presence service throttles updates, and a zoom drag
/// changes the state on every frame.
const MIN_INTERVAL: Duration = Duration::from_secs(2);

/// Decides when the shell hears about a change.
#[derive(Default)]
pub struct Tracker {
    /// The state last reported, and when.
    last: Option<(PresenceInfo, Instant)>,
    /// A change that came too early: reported as soon as the interval has passed.
    pending: Option<PresenceInfo>,
}

impl Tracker {
    /// The state to report now, if any.
    pub fn due(&mut self, info: &PresenceInfo, now: Instant) -> Option<PresenceInfo> {
        if self.last.as_ref().is_some_and(|(last, _)| last == info) && self.pending.is_none() {
            return None;
        }
        let ready = self.last.as_ref().is_none_or(|(_, at)| now.duration_since(*at) >= MIN_INTERVAL);
        if !ready {
            self.pending = Some(info.clone());
            return None;
        }
        let out = self.pending.take().unwrap_or_else(|| info.clone());
        self.last = Some((out.clone(), now));
        Some(out)
    }
}

/// The state as the shell should show it.
pub fn info(app: &PhotocraftApp) -> PresenceInfo {
    let active = app.session.active();
    // The zoom belongs to the document's view (per document, in the tab order).
    let zoom = app.session.active_index().and_then(|index| app.ui.views.get(index)).map(|view| view.zoom).unwrap_or(1.0);
    // The file name is private unless the user asks for it, and a path never leaves the
    // machine (the family's rule for the status, #4148).
    let named = app.session.prefs().general.discord_rich_presence_file_name;
    PresenceInfo {
        document: match (active.as_ref(), named) {
            (Some(state), true) => base_name(&state.doc.name),
            _ => String::new(),
        },
        editing: active.is_some(),
        tool: app.ui.tool.label().trim_end_matches(" Tool").to_string(),
        size: active.map(|d| (d.doc.size.width, d.doc.size.height)),
        zoom,
        dirty: active.is_some_and(|d| d.is_dirty()),
    }
}

/// The name without any directory part: a path never leaves the machine.
fn base_name(name: &str) -> String {
    let cut = name.rfind(&['/', '\\'][..]).map(|index| index + 1).unwrap_or(0);
    name[cut..].to_string()
}

/// Tell the shell what changed, when the user asked for it (Preferences ▸ General ▸ Discord
/// Rich Presence). Without the preference, or without a shell that publishes it, nothing happens.
pub fn report(app: &mut PhotocraftApp) {
    if !app.session.prefs().general.discord_rich_presence {
        return;
    }
    let Some(hook) = app.services.presence.clone() else { return };
    let info = info(app);
    if let Some(info) = app.presence.due(&info, Instant::now()) {
        hook(info);
    }
}

#[cfg(test)]
mod tests {
    use super::{MIN_INTERVAL, PresenceInfo, Tracker};

    fn info(document: &str) -> PresenceInfo {
        PresenceInfo { document: document.into(), editing: !document.is_empty(), tool: "Brush".into(), size: Some((10, 20)), zoom: 1.0, dirty: false }
    }

    #[test]
    fn the_first_state_is_reported() {
        let mut tracker = Tracker::default();
        let now = std::time::Instant::now();
        assert_eq!(tracker.due(&info("a.psd"), now), Some(info("a.psd")));
    }

    #[test]
    fn the_same_state_is_not_reported_again() {
        let mut tracker = Tracker::default();
        let now = std::time::Instant::now();
        assert!(tracker.due(&info("a.psd"), now).is_some());
        assert_eq!(tracker.due(&info("a.psd"), now + MIN_INTERVAL), None);
    }

    #[test]
    fn a_change_inside_the_interval_is_reported_later() {
        let mut tracker = Tracker::default();
        let now = std::time::Instant::now();
        assert!(tracker.due(&info("a.psd"), now).is_some());
        // Too early: the change waits instead of being dropped.
        assert_eq!(tracker.due(&info("b.psd"), now + MIN_INTERVAL / 2), None);
        let later = tracker.due(&info("b.psd"), now + MIN_INTERVAL);
        assert_eq!(later, Some(info("b.psd")), "the pending change is reported once the interval has passed");
    }

    /// An app with a shell that records what it hears.
    fn app_with_shell() -> (crate::PhotocraftApp, std::sync::Arc<std::sync::Mutex<Vec<PresenceInfo>>>) {
        use std::sync::{Arc, Mutex};
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let heard: Arc<Mutex<Vec<PresenceInfo>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&heard);
        app.services.presence = Some(Arc::new(move |info| {
            sink.lock().unwrap_or_else(|error| error.into_inner()).push(info);
        }));
        (app, heard)
    }

    fn open(app: &mut crate::PhotocraftApp, name: &str) {
        app.session.add_document(
            photocraft_doc::Document::new(name, photocraft_doc::Size::new(4, 5), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8),
            None,
        );
    }

    /// The shell hears about the open document, but only when the user asked for it.
    #[test]
    fn the_shell_hears_about_the_document_when_the_user_asked_for_it() {
        let (mut app, heard) = app_with_shell();

        // The preference is off: the shell hears nothing.
        crate::presence::report(&mut app);
        assert!(heard.lock().unwrap_or_else(|error| error.into_inner()).is_empty());

        app.session.edit_prefs(|p| p.general.discord_rich_presence = true);
        open(&mut app, "foo.psd");
        crate::presence::report(&mut app);

        let heard = heard.lock().unwrap_or_else(|error| error.into_inner());
        assert_eq!(heard.len(), 1, "the shell is told once: {heard:?}");
        assert!(heard[0].editing, "a document is open");
        assert_eq!(heard[0].size, Some((4, 5)));
        assert_eq!(heard[0].tool, "Brush", "the active tool, without the \"Tool\" suffix");
    }

    /// The file name is private unless the user asks for it (#4148).
    #[test]
    fn the_file_name_is_shown_only_when_the_user_asks_for_it() {
        let (mut app, heard) = app_with_shell();
        app.session.edit_prefs(|p| {
            p.general.discord_rich_presence = true;
            p.general.discord_rich_presence_file_name = true;
        });
        open(&mut app, "/home/someone/secret/holiday.psd");
        crate::presence::report(&mut app);

        let heard = heard.lock().unwrap_or_else(|error| error.into_inner());
        assert_eq!(heard.len(), 1, "the shell is told once: {heard:?}");
        assert_eq!(heard[0].document, "holiday.psd", "a path never leaves the machine");
    }
}
