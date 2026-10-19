//! Discord Rich Presence (#4148): show what PhotoCraft is doing on the user's Discord status.
//!
//! Discord's desktop client listens on a local socket: an app connects, says which Discord
//! application it is, and then sends activity frames. The socket is a Unix socket on Linux and
//! macOS and a named pipe on Windows, and Discord creates it when it runs — with no Discord
//! running, there is nothing to talk to and PhotoCraft carries on without it.
//!
//! The Discord application id belongs to the application in the Discord developer portal. The
//! environment variable `PHOTOCRAFT_DISCORD_APP_ID` wins over the constant below, so a build
//! (or a test) can point at another application without touching the source.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use photocraft_ui_egui::presence::{Hook, PresenceInfo};
use serde_json::{Value, json};

/// The Discord application id PhotoCraft publishes under. The maintainer fills in the id of the
/// application in the Discord developer portal; an empty id leaves the feature off, and
/// `PHOTOCRAFT_DISCORD_APP_ID` overrides it.
pub const DISCORD_APP_ID: &str = "";

/// The large image shown on the activity card: the key of an asset uploaded to the Discord
/// application. Empty means the card shows the application's own icon.
pub const LARGE_IMAGE_KEY: &str = "";

/// Discord's IPC opcodes.
const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_PING: u32 = 3;

/// How long to wait before looking for Discord again when it isn't running.
const RETRY: Duration = Duration::from_secs(15);

/// How often to ping Discord, so the connection stays open.
const PING_INTERVAL: Duration = Duration::from_secs(30);

/// Discord's limit for the activity's details and state.
const MAX_TEXT: usize = 128;

/// What to show on the Discord status.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Activity {
    /// The first line under the application's name.
    pub details: String,
    /// The second line.
    pub state: String,
    /// The big text on the activity card.
    pub large_text: String,
    /// When the session started (Unix seconds).
    pub start: i64,
    /// The asset key of the card's image ("" for the application's own icon).
    pub large_image: String,
}

/// A place Discord's IPC may listen.
#[derive(Clone, Debug, PartialEq)]
pub enum Endpoint {
    /// A Unix socket (Linux, macOS).
    Unix(PathBuf),
    /// A Windows named pipe.
    Pipe(String),
}

/// The endpoints to try, in order. `PHOTOCRAFT_DISCORD_IPC` names one socket, which is how the
/// tests point the client at a stand-in for Discord.
pub fn endpoints() -> Vec<Endpoint> {
    if let Some(path) = std::env::var_os("PHOTOCRAFT_DISCORD_IPC") {
        return vec![Endpoint::Unix(PathBuf::from(path))];
    }
    if cfg!(windows) {
        return (0..10).map(|i| Endpoint::Pipe(format!(r"\\.\pipe\discord-ipc-{i}"))).collect();
    }
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::var_os("TMPDIR") {
        dirs.push(PathBuf::from(dir));
    }
    dirs.push(PathBuf::from("/tmp"));
    let mut out = Vec::new();
    for dir in dirs {
        for i in 0..10 {
            out.push(Endpoint::Unix(dir.join(format!("discord-ipc-{i}"))));
        }
    }
    out
}

/// One IPC frame: the opcode, the payload's length, then the payload.
fn frame(op: u32, payload: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// The handshake that names the Discord application.
fn handshake(app_id: &str) -> Vec<u8> {
    frame(OP_HANDSHAKE, &json!({"v": 1, "client_id": app_id}))
}

/// The `SET_ACTIVITY` frame for what the user is doing.
fn set_activity(pid: u32, activity: &Activity) -> Vec<u8> {
    let mut body = serde_json::Map::new();
    if !activity.details.is_empty() {
        body.insert("details".into(), json!(activity.details));
    }
    if !activity.state.is_empty() {
        body.insert("state".into(), json!(activity.state));
    }
    if !activity.large_text.is_empty() {
        body.insert("large_text".into(), json!(activity.large_text));
    }
    if !activity.large_image.is_empty() {
        body.insert("assets".into(), json!({"large_image": activity.large_image}));
    }
    body.insert("timestamps".into(), json!({"start": activity.start}));
    frame(OP_FRAME, &json!({"cmd": "SET_ACTIVITY", "args": {"pid": pid, "activity": Value::Object(body)}}))
}

/// The Unix time in seconds, or 0 when the clock is before 1970.
fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Shorten a line to Discord's limit, on a character boundary.
fn clipped(text: &str) -> String {
    if text.chars().count() <= MAX_TEXT {
        return text.to_string();
    }
    text.chars().take(MAX_TEXT - 1).collect::<String>() + "…"
}

/// What the user is doing, as the Discord status shows it.
pub fn activity(info: &PresenceInfo, start: i64) -> Activity {
    let document = info.document.trim();
    // Unsaved changes get the asterisk the window title uses. The name is empty when the user
    // keeps it private, and then the status says only that a document is open.
    let details = if !info.editing {
        "No document open".to_string()
    } else if document.is_empty() {
        "Editing a document".to_string()
    } else if info.dirty {
        format!("*{document}")
    } else {
        document.to_string()
    };
    let mut state = info.tool.clone();
    if let Some((width, height)) = info.size {
        state.push_str(&format!(" · {width} × {height}"));
    }
    state.push_str(&format!(" · {}%", (info.zoom * 100.0).round() as i64));
    Activity { details: clipped(&details), state: clipped(&state), large_text: clipped(document), start, large_image: LARGE_IMAGE_KEY.to_string() }
}

/// The Discord application id: the environment variable wins over the constant.
fn app_id() -> String {
    std::env::var("PHOTOCRAFT_DISCORD_APP_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DISCORD_APP_ID.trim().to_string())
}

/// The publisher, started with the first report: PhotoCraft holds no Discord connection while
/// the preference is off.
static PUBLISHER: std::sync::LazyLock<Option<Publisher>> = std::sync::LazyLock::new(|| {
    let app_id = app_id();
    if app_id.is_empty() {
        return None;
    }
    Publisher::start(app_id)
});

/// The hook the UI calls when the user turned the feature on. `None` when no Discord
/// application is configured, so the UI has nothing to call.
pub fn install() -> Option<Hook> {
    if app_id().is_empty() {
        return None;
    }
    let start = unix_now();
    Some(Arc::new(move |info: PresenceInfo| {
        if let Some(publisher) = PUBLISHER.as_ref() {
            publisher.set(activity(&info, start));
        }
    }))
}

/// Publishes what PhotoCraft is doing to the running Discord client.
pub struct Publisher {
    tx: tokio::sync::mpsc::Sender<Activity>,
}

impl Publisher {
    /// Start the publisher thread. It looks for Discord, and keeps looking until it finds it.
    pub fn start(app_id: String) -> Option<Publisher> {
        Publisher::start_at(app_id, endpoints())
    }

    /// Start the publisher thread against the given endpoints.
    fn start_at(app_id: String, endpoints: Vec<Endpoint>) -> Option<Publisher> {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        std::thread::Builder::new().name("photocraft-discord".into()).spawn(move || run(app_id, endpoints, rx)).ok()?;
        Some(Publisher { tx })
    }

    /// Publish what the user is doing. Never blocks the caller: a full queue drops the update.
    pub fn set(&self, activity: Activity) {
        let _ = self.tx.try_send(activity);
    }
}

/// A connected Discord client.
struct Connection {
    stream: BoxStream,
    app_id: String,
}

/// The socket Discord talks over: a Unix socket or a named pipe, both readable and writable.
trait Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin> Stream for T {}

type BoxStream = Box<dyn Stream>;

impl Connection {
    /// Say which Discord application we are.
    async fn open(mut self) -> std::io::Result<Connection> {
        use tokio::io::AsyncWriteExt;
        self.stream.write_all(&handshake(&self.app_id)).await?;
        self.stream.flush().await?;
        Ok(self)
    }

    /// Publish what the user is doing.
    async fn set_activity(&mut self, activity: &Activity) -> std::io::Result<()> {
        use tokio::io::AsyncWriteExt;
        self.stream.write_all(&set_activity(std::process::id(), activity)).await?;
        self.stream.flush().await
    }

    /// Keep the connection open.
    async fn ping(&mut self) -> std::io::Result<()> {
        use tokio::io::AsyncWriteExt;
        self.stream.write_all(&frame(OP_PING, &json!({}))).await?;
        self.stream.flush().await
    }
}

/// Connect to the first endpoint that answers.
async fn connect(endpoints: &[Endpoint]) -> std::io::Result<BoxStream> {
    let mut last = std::io::Error::new(std::io::ErrorKind::NotFound, "no Discord IPC socket");
    for endpoint in endpoints {
        match connect_one(endpoint).await {
            Ok(stream) => return Ok(stream),
            Err(error) => last = error,
        }
    }
    Err(last)
}

/// Connect to one endpoint.
#[cfg(unix)]
async fn connect_one(endpoint: &Endpoint) -> std::io::Result<BoxStream> {
    let Endpoint::Unix(path) = endpoint else { return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a Unix socket")) };
    Ok(Box::new(tokio::net::UnixStream::connect(path).await?))
}

/// Connect to one endpoint.
#[cfg(windows)]
async fn connect_one(endpoint: &Endpoint) -> std::io::Result<BoxStream> {
    use tokio::net::windows::named_pipe::ClientOptions;
    let Endpoint::Pipe(name) = endpoint else { return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a named pipe")) };
    Ok(Box::new(ClientOptions::new().open(name).await?))
}

/// The publisher thread: find Discord, publish, and start over when the connection ends.
fn run(app_id: String, endpoints: Vec<Endpoint>, mut rx: tokio::sync::mpsc::Receiver<Activity>) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            log::warn!("discord: no runtime for the presence client: {error}");
            return;
        }
    };
    runtime.block_on(async move {
        let mut last: Option<Activity> = None;
        let mut warned = false;
        loop {
            match connect(&endpoints).await {
                Ok(stream) => {
                    let connection = Connection { stream, app_id: app_id.clone() };
                    let mut connection = match connection.open().await {
                        Ok(connection) => connection,
                        Err(error) => {
                            if !warned {
                                log::warn!("discord: the connection failed: {error}");
                                warned = true;
                            }
                            tokio::time::sleep(RETRY).await;
                            continue;
                        }
                    };
                    warned = false;
                    if let Some(activity) = last.take() {
                        let _ = connection.set_activity(&activity).await;
                    }
                    loop {
                        tokio::select! {
                            message = rx.recv() => match message {
                                Some(activity) => {
                                    if let Err(error) = connection.set_activity(&activity).await {
                                        log::warn!("discord: the connection ended ({error}); is the application id right?");
                                        break;
                                    }
                                    last = Some(activity);
                                }
                                // The publisher was dropped: the app is closing.
                                None => return,
                            },
                            _ = tokio::time::sleep(PING_INTERVAL) => {
                                if let Err(error) = connection.ping().await {
                                    log::warn!("discord: the connection ended ({error}); is the application id right?");
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    if !warned {
                        log::debug!("discord: no client to talk to yet: {error}");
                        warned = true;
                    }
                }
            }
            // Discord may not be running yet; look again in a while.
            tokio::time::sleep(RETRY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{Activity, Endpoint, MAX_TEXT, Publisher, activity, endpoints, frame, handshake, set_activity};
    use photocraft_ui_egui::presence::PresenceInfo;

    #[test]
    fn a_frame_carries_its_opcode_and_length() {
        let bytes = frame(1, &serde_json::json!({"a": 1}));
        assert_eq!(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]), 1);
        let len = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        assert_eq!(len, bytes.len() - 8);
        assert_eq!(&bytes[8..], b"{\"a\":1}");
    }

    #[test]
    fn the_handshake_names_the_application() {
        let bytes = handshake("12345");
        let body = String::from_utf8(bytes[8..].to_vec()).unwrap();
        assert!(body.contains("\"v\":1"), "{body}");
        assert!(body.contains("\"client_id\":\"12345\""), "{body}");
    }

    #[test]
    fn the_activity_frame_carries_what_the_user_is_doing() {
        let activity = Activity {
            details: "sunset.jpg".into(),
            state: "Brush · 800 × 600 · 100%".into(),
            large_text: "sunset.jpg".into(),
            start: 1_700_000_000,
            large_image: "logo".into(),
        };
        let bytes = set_activity(42, &activity);
        let body: serde_json::Value = serde_json::from_slice(&bytes[8..]).unwrap();
        assert_eq!(body["cmd"], "SET_ACTIVITY");
        assert_eq!(body["args"]["pid"], 42);
        assert_eq!(body["args"]["activity"]["details"], "sunset.jpg");
        assert_eq!(body["args"]["activity"]["state"], "Brush · 800 × 600 · 100%");
        assert_eq!(body["args"]["activity"]["large_text"], "sunset.jpg");
        assert_eq!(body["args"]["activity"]["timestamps"]["start"], 1_700_000_000);
        assert_eq!(body["args"]["activity"]["assets"]["large_image"], "logo");
    }

    #[test]
    fn an_activity_without_a_document_says_so() {
        let info = PresenceInfo { tool: "Move".into(), ..PresenceInfo::default() };
        let activity = activity(&info, 0);
        assert_eq!(activity.details, "No document open");
        assert_eq!(activity.state, "Move · 0%");
        assert_eq!(activity.large_text, "");
    }

    /// The file name is private unless the user asked for it (#4148).
    #[test]
    fn a_private_file_name_is_not_published() {
        let info = PresenceInfo { editing: true, tool: "Brush".into(), size: Some((800, 600)), zoom: 1.0, ..PresenceInfo::default() };
        let activity = activity(&info, 0);
        assert_eq!(activity.details, "Editing a document");
        assert_eq!(activity.large_text, "", "the name is not on the card either");
        assert_eq!(activity.state, "Brush · 800 × 600 · 100%");
    }

    #[test]
    fn unsaved_changes_get_the_window_titles_asterisk() {
        let info = PresenceInfo { document: "sunset.jpg".into(), editing: true, tool: "Brush".into(), dirty: true, ..PresenceInfo::default() };
        assert_eq!(activity(&info, 0).details, "*sunset.jpg");
    }

    #[test]
    fn long_lines_are_clipped_to_discords_limit() {
        let info = PresenceInfo { document: "x".repeat(400), editing: true, tool: "Brush".into(), size: Some((10, 10)), zoom: 1.0, dirty: false };
        let activity = activity(&info, 0);
        assert_eq!(activity.details.chars().count(), MAX_TEXT);
        assert_eq!(activity.large_text.chars().count(), MAX_TEXT);
    }

    #[test]
    fn the_endpoints_cover_the_ten_discord_sockets() {
        let endpoints = endpoints();
        assert!(!endpoints.is_empty());
        if cfg!(windows) {
            assert!(endpoints.iter().any(|e| matches!(e, Endpoint::Pipe(name) if name.contains("discord-ipc-0"))));
        } else {
            assert!(endpoints.iter().any(|e| matches!(e, Endpoint::Unix(path) if path.to_string_lossy().contains("discord-ipc-0"))));
        }
    }

    /// The client says hello and publishes what the user is doing, over the socket Discord
    /// would listen on.
    #[cfg(unix)]
    #[test]
    fn the_client_says_hello_and_publishes_the_activity() {
        let discord = FakeDiscord::start("client");
        let publisher = Publisher::start_at("4242".into(), discord.endpoint()).expect("the publisher starts");
        publisher.set(Activity {
            details: "sunset.jpg".into(),
            state: "Brush · 800 × 600 · 100%".into(),
            large_text: "sunset.jpg".into(),
            start: 7,
            large_image: String::new(),
        });

        let hello = discord.next();
        let published = discord.next();

        assert_eq!(hello.0, 0, "the first frame is the handshake");
        assert_eq!(hello.1["client_id"], "4242");
        assert_eq!(published.0, 1, "the second frame is the activity");
        assert_eq!(published.1["cmd"], "SET_ACTIVITY");
        assert_eq!(published.1["args"]["activity"]["details"], "sunset.jpg");
        assert_eq!(published.1["args"]["activity"]["state"], "Brush · 800 × 600 · 100%");
        assert_eq!(published.1["args"]["activity"]["timestamps"]["start"], 7);
    }

    /// The whole chain: the UI reports what the user is doing, the shell turns it into an
    /// activity, and the client publishes it over the socket.
    #[cfg(unix)]
    #[test]
    fn the_shell_publishes_what_the_ui_reports() {
        use photocraft_ui_egui::presence::{Hook, PresenceInfo};

        let discord = FakeDiscord::start("chain");
        let publisher = std::sync::Arc::new(Publisher::start_at("4242".into(), discord.endpoint()).expect("the publisher starts"));
        let hook: Hook = {
            let publisher = std::sync::Arc::clone(&publisher);
            std::sync::Arc::new(move |info: PresenceInfo| {
                publisher.set(activity(&info, 0));
            })
        };
        let mut app = photocraft_ui_egui::PhotocraftApp::new(
            photocraft_engine::Session::new(),
            photocraft_ui_egui::Services { presence: Some(hook), ..Default::default() },
        );
        app.session.edit_prefs(|p| p.general.discord_rich_presence = true);
        app.session.add_document(
            photocraft_doc::Document::new("sunset.jpg", photocraft_doc::Size::new(800, 600), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8),
            None,
        );
        photocraft_ui_egui::presence::report(&mut app);

        let hello = discord.next();
        let published = discord.next();
        assert_eq!(hello.0, 0, "the first frame is the handshake");
        assert_eq!(hello.1["client_id"], "4242");
        assert_eq!(published.1["cmd"], "SET_ACTIVITY");
        // The file name is private unless the user asks for it (#4148).
        assert_eq!(published.1["args"]["activity"]["details"], "Editing a document");
        assert_eq!(published.1["args"]["activity"]["state"], "Brush · 800 × 600 · 100%");
    }

    /// A stand-in for Discord: a socket that records the frames the client sends.
    #[cfg(unix)]
    struct FakeDiscord {
        path: std::path::PathBuf,
        frames: std::sync::mpsc::Receiver<(u32, serde_json::Value)>,
    }

    #[cfg(unix)]
    impl FakeDiscord {
        fn start(label: &str) -> FakeDiscord {
            let path = std::env::temp_dir().join(format!("photocraft-discord-{label}-{}.sock", std::process::id()));
            let _ = std::fs::remove_file(&path);
            let listener = std::os::unix::net::UnixListener::bind(&path).expect("the test socket");
            let (tx, frames) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                if let Ok((mut stream, _)) = listener.accept() {
                    for _ in 0..2 {
                        if tx.send(read_frame(&mut stream)).is_err() {
                            break;
                        }
                    }
                }
            });
            FakeDiscord { path, frames }
        }

        /// The endpoint the client should talk to.
        fn endpoint(&self) -> Vec<Endpoint> {
            vec![Endpoint::Unix(self.path.clone())]
        }

        /// The next frame the client sent.
        fn next(&self) -> (u32, serde_json::Value) {
            self.frames.recv_timeout(std::time::Duration::from_secs(20)).expect("the client talks to the socket")
        }
    }

    /// Read one frame: the opcode and the payload.
    #[cfg(unix)]
    fn read_frame(stream: &mut std::os::unix::net::UnixStream) -> (u32, serde_json::Value) {
        use std::io::Read;
        let mut head = [0u8; 8];
        stream.read_exact(&mut head).expect("the frame header");
        let op = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).expect("the frame body");
        (op, serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null))
    }
}
