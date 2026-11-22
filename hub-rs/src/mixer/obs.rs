//! OBS Studio, connected through the obs-websocket plugin (protocol version 4).
//!
//! See <https://github.com/obsproject/obs-websocket/blob/4.x-current/docs/generated/protocol.md>.
//!
//! The plugin sends events (`SwitchScenes`, `TransitionBegin`, ...) as JSON over a websocket and answers
//! requests (`GetSceneList`, `GetPreviewScene`, ...). Scenes that are embedded into a visible scene are
//! live, too, so the scene list is queried again whenever scenes or their items change.
//!
//! Servers that require a password are not supported (there is no password setting).

use super::config::{Endpoint, ObsConfig};
use super::connectors::{Connector, TaskGuard};
use crate::domain::channel::Channel;
use crate::hub::Hub;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info};

const RECONNECT_DELAY: Duration = Duration::from_secs(1);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// What OBS is doing, and what that means for the tallies.
#[derive(Debug, Default)]
struct ObsState {
    /// "always", "stream", "record" or "streamOrRecord"
    live_mode: String,
    /// scene name -> scenes that are visible inside of it
    embedded: HashMap<String, Vec<String>>,
    program: Vec<String>,
    preview: Vec<String>,
    streaming: bool,
    recording: bool,
}

/// What the connection has to do after a message was handled.
#[derive(Debug, Default, PartialEq)]
struct Reaction {
    /// publish program and preview
    changed: bool,
    refresh_scenes: bool,
    refresh_preview: bool,
}

impl Reaction {
    fn changed() -> Self {
        Reaction { changed: true, ..Default::default() }
    }
}

fn str_field<'a>(data: &'a Value, key: &str) -> Option<&'a str> {
    data.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

impl ObsState {
    fn new(live_mode: &str) -> Self {
        ObsState {
            live_mode: live_mode.to_string(),
            ..Default::default()
        }
    }

    /// Should the program be shown as preview, because nothing is "live" yet?
    fn program_is_preview(&self) -> bool {
        match self.live_mode.as_str() {
            "record" => !self.recording,
            "stream" => !self.streaming,
            "streamOrRecord" => !self.streaming && !self.recording,
            _ => false, // "always"
        }
    }

    /// The scenes that are on program and in preview, including the ones embedded into them.
    fn output(&self) -> (Vec<String>, Vec<String>) {
        let expand = |scenes: &[String]| {
            let mut all = Vec::new();
            for scene in scenes {
                all.push(scene.clone());
                if let Some(embedded) = self.embedded.get(scene) {
                    all.extend(embedded.iter().cloned());
                }
            }
            all
        };
        let programs = expand(&self.program);
        if self.program_is_preview() {
            (Vec::new(), programs)
        } else {
            (programs, expand(&self.preview))
        }
    }

    /// An event of OBS.
    fn event(&mut self, kind: &str, data: &Value) -> Reaction {
        match kind {
            "SwitchScenes" => {
                self.program = str_field(data, "scene-name").map(String::from).into_iter().collect();
                Reaction::changed()
            }
            "ScenesChanged" | "SceneItemRemoved" | "SceneItemAdded" | "SceneItemVisibilityChanged"
            | "SceneCollectionChanged" | "SceneCollectionListChanged" => Reaction {
                refresh_scenes: true,
                ..Default::default()
            },
            "TransitionBegin" => {
                self.program = ["from-scene", "to-scene"]
                    .iter()
                    .filter_map(|k| str_field(data, k))
                    .map(String::from)
                    .collect();
                Reaction::changed()
            }
            "TransitionEnd" => {
                self.program = str_field(data, "to-scene").map(String::from).into_iter().collect();
                Reaction::changed()
            }
            "PreviewSceneChanged" => {
                self.preview = str_field(data, "scene-name").map(String::from).into_iter().collect();
                Reaction::changed()
            }
            "StudioModeSwitched" => {
                if data.get("new-state").and_then(Value::as_bool).unwrap_or(false) {
                    Reaction { refresh_preview: true, ..Default::default() }
                } else {
                    self.preview.clear();
                    Reaction::changed()
                }
            }
            "StreamStarting" => self.set_streaming(true),
            "StreamStopped" => self.set_streaming(false),
            "RecordingStarting" | "RecordingResumed" => self.set_recording(true),
            "RecordingStopped" | "RecordingPaused" => self.set_recording(false),
            "StreamStatus" => {
                self.streaming = data.get("streaming").and_then(Value::as_bool).unwrap_or(false);
                self.recording = data.get("recording").and_then(Value::as_bool).unwrap_or(false);
                Reaction::changed()
            }
            _ => Reaction::default(),
        }
    }

    fn set_streaming(&mut self, on: bool) -> Reaction {
        self.streaming = on;
        Reaction::changed()
    }

    fn set_recording(&mut self, on: bool) -> Reaction {
        self.recording = on;
        Reaction::changed()
    }

    /// The answer to `GetSceneList`. Returns the scenes as channels.
    fn scene_list(&mut self, data: &Value) -> (Vec<Channel>, Reaction) {
        let scenes = data.get("scenes").and_then(Value::as_array).cloned().unwrap_or_default();
        self.embedded.clear();
        let mut channels = Vec::new();
        for scene in &scenes {
            let Some(name) = scene.get("name").and_then(Value::as_str) else { continue };
            let embedded = scene
                .get("sources")
                .and_then(Value::as_array)
                .map(|sources| {
                    sources
                        .iter()
                        .filter(|s| {
                            s.get("type").and_then(Value::as_str) == Some("scene")
                                && s.get("render").and_then(Value::as_bool) == Some(true)
                        })
                        .filter_map(|s| s.get("name").and_then(Value::as_str).map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            self.embedded.insert(name.to_string(), embedded);
            channels.push(Channel::new(name, Some(name.to_string())));
        }
        if let Some(current) = str_field(data, "current-scene") {
            self.program = vec![current.to_string()];
        }
        (channels, Reaction::changed())
    }

    /// The answer to `GetPreviewScene`. Fails (and is ignored) while the studio mode is off.
    fn preview_scene(&mut self, data: &Value) -> Reaction {
        match str_field(data, "name") {
            Some(name) => {
                self.preview = vec![name.to_string()];
                Reaction::changed()
            }
            None => Reaction::default(),
        }
    }

    /// The answer to `GetStreamingStatus`.
    fn streaming_status(&mut self, data: &Value) -> Reaction {
        let flag = |k: &str| data.get(k).and_then(Value::as_bool).unwrap_or(false);
        self.streaming = flag("streaming");
        self.recording = flag("recording") && !flag("recording-paused");
        Reaction::changed()
    }
}

pub struct ObsConnector {
    hub: Arc<Hub>,
    config: ObsConfig,
    connected: Arc<AtomicBool>,
    task: Option<TaskGuard>,
}

impl ObsConnector {
    pub fn new(hub: Arc<Hub>, config: ObsConfig) -> Self {
        ObsConnector {
            hub,
            config,
            connected: Arc::new(AtomicBool::new(false)),
            task: None,
        }
    }
}

impl Connector for ObsConnector {
    fn connect(&mut self) {
        self.hub.notify_program_preview_changed(None, None);
        let hub = Arc::clone(&self.hub);
        let config = self.config.clone();
        let connected = Arc::clone(&self.connected);
        self.task = Some(TaskGuard::spawn(async move {
            run(hub, config, connected).await
        }));
    }

    fn disconnect(&mut self) {
        info!("Disconnecting from OBS");
        self.task = None; // aborts the task
        self.connected.store(false, Ordering::SeqCst);
        self.hub.notify_mixer_is_disconnected();
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

/// Connects again and again until the task is aborted.
async fn run(hub: Arc<Hub>, config: ObsConfig, connected: Arc<AtomicBool>) {
    let mut last_error = String::new();
    loop {
        let Endpoint { ip, port } = &config.endpoint;
        info!("Connecting to OBS at {ip}:{port}");
        let result = session(&hub, &config, &connected).await;
        connected.store(false, Ordering::SeqCst);
        hub.notify_mixer_is_disconnected();
        match result {
            Ok(()) => {
                error!("Connection to OBS lost");
                last_error.clear();
            }
            Err(e) => {
                // do not repeat the same message every second while OBS is away
                if e != last_error {
                    error!("{e}");
                    last_error = e;
                }
            }
        }
        sleep(RECONNECT_DELAY).await;
    }
}

/// What a request was for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Request {
    AuthRequired,
    SceneList,
    PreviewScene,
    StreamingStatus,
}

impl Request {
    fn name(self) -> &'static str {
        match self {
            Request::AuthRequired => "GetAuthRequired",
            Request::SceneList => "GetSceneList",
            Request::PreviewScene => "GetPreviewScene",
            Request::StreamingStatus => "GetStreamingStatus",
        }
    }
}

/// Sends requests and remembers what they were for.
struct Requests {
    next_id: u64,
    pending: HashMap<String, Request>,
}

impl Requests {
    fn message(&mut self, request: Request) -> Message {
        self.next_id += 1;
        let id = self.next_id.to_string();
        self.pending.insert(id.clone(), request);
        Message::text(json!({ "request-type": request.name(), "message-id": id }).to_string())
    }
}

/// One connection. Returns when it was closed or failed.
async fn session(hub: &Arc<Hub>, config: &ObsConfig, connected: &AtomicBool) -> Result<(), String> {
    let Endpoint { ip, port } = &config.endpoint;
    let url = format!("ws://{ip}:{port}");
    let (mut socket, _) = timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(url))
        .await
        .map_err(|_| format!("Connection to OBS at {ip}:{port} timed out"))?
        .map_err(|e| format!("Could not connect to OBS at {ip}:{port}: {e}"))?;
    debug!("Websocket connection to {ip}:{port} established");

    let failed = |e: tokio_tungstenite::tungstenite::Error| format!("Connection to OBS failed: {e}");
    let mut requests = Requests { next_id: 0, pending: HashMap::new() };
    let mut state = ObsState::new(&config.live_mode);

    socket.send(requests.message(Request::AuthRequired)).await.map_err(failed)?;

    while let Some(message) = socket.next().await {
        let message = message.map_err(failed)?;
        let Message::Text(text) = message else {
            if matches!(message, Message::Close(_)) {
                return Ok(());
            }
            continue; // ping, pong
        };
        let Ok(data) = serde_json::from_str::<Value>(&text) else {
            debug!("Ignoring a message of OBS that is not JSON");
            continue;
        };

        let mut reaction = Reaction::default();
        if let Some(kind) = data.get("update-type").and_then(Value::as_str) {
            reaction = state.event(kind, &data);
        } else if let Some(id) = data.get("message-id").and_then(Value::as_str) {
            let Some(request) = requests.pending.remove(id) else { continue };
            let ok = data.get("status").and_then(Value::as_str) == Some("ok");
            match request {
                Request::AuthRequired if !ok => {
                    return Err("OBS did not answer the authentication check".into());
                }
                Request::AuthRequired => {
                    if data.get("authRequired").and_then(Value::as_bool) == Some(true) {
                        return Err("OBS requires a password, which is not supported".into());
                    }
                    info!("Connected to OBS");
                    connected.store(true, Ordering::SeqCst);
                    hub.notify_mixer_is_connected();
                    for request in [Request::SceneList, Request::PreviewScene, Request::StreamingStatus] {
                        socket.send(requests.message(request)).await.map_err(failed)?;
                    }
                }
                // OBS reports an error for the preview scene while the studio mode is off
                _ if !ok => debug!("OBS could not answer {}: {}", request.name(), data["error"]),
                Request::SceneList => {
                    let (channels, r) = state.scene_list(&data);
                    hub.notify_channels(Some(channels));
                    reaction = r;
                }
                Request::PreviewScene => reaction = state.preview_scene(&data),
                Request::StreamingStatus => reaction = state.streaming_status(&data),
            }
        }

        if reaction.refresh_scenes {
            socket.send(requests.message(Request::SceneList)).await.map_err(failed)?;
        }
        if reaction.refresh_preview {
            socket.send(requests.message(Request::PreviewScene)).await.map_err(failed)?;
        }
        if reaction.changed && connected.load(Ordering::SeqCst) {
            let (programs, previews) = state.output();
            hub.notify_program_preview_changed(Some(programs), Some(previews));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appconfig::AppConfig;
    use crate::hub::HubOptions;
    use tokio::net::TcpListener;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn state(mode: &str) -> ObsState {
        ObsState::new(mode)
    }

    fn scenes() -> Value {
        json!({
            "current-scene": "Main",
            "scenes": [
                {"name": "Main", "sources": [
                    {"name": "Camera", "type": "input", "render": true},
                    {"name": "Lower third", "type": "scene", "render": true},
                    {"name": "Hidden", "type": "scene", "render": false},
                ]},
                {"name": "Lower third", "sources": []},
                {"name": "Hidden", "sources": []},
                {"name": "Break", "sources": []},
            ]
        })
    }

    #[test]
    fn scene_list_gives_channels_program_and_embedded_scenes() {
        let mut s = state("always");
        let (channels, r) = s.scene_list(&scenes());
        assert!(r.changed);
        assert_eq!(
            channels.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["Main", "Lower third", "Hidden", "Break"]
        );
        assert_eq!(channels[3].name.as_deref(), Some("Break"));
        // only visible scenes are embedded
        assert_eq!(s.output(), (names(&["Main", "Lower third"]), vec![]));
    }

    #[test]
    fn switching_scenes_and_transitions() {
        let mut s = state("always");
        s.scene_list(&scenes());
        s.event("SwitchScenes", &json!({"scene-name": "Break"}));
        assert_eq!(s.output().0, names(&["Break"]));

        // both scenes are on program during a transition
        s.event("TransitionBegin", &json!({"from-scene": "Break", "to-scene": "Main"}));
        assert_eq!(s.output().0, names(&["Break", "Main", "Lower third"]));
        // a transition without a source scene
        s.event("TransitionBegin", &json!({"to-scene": "Break"}));
        assert_eq!(s.output().0, names(&["Break"]));
        s.event("TransitionEnd", &json!({"to-scene": "Main"}));
        assert_eq!(s.output().0, names(&["Main", "Lower third"]));
    }

    #[test]
    fn preview_and_studio_mode() {
        let mut s = state("always");
        s.scene_list(&scenes());
        s.event("PreviewSceneChanged", &json!({"scene-name": "Main"}));
        assert_eq!(s.output().1, names(&["Main", "Lower third"]));

        // leaving the studio mode removes the preview
        let r = s.event("StudioModeSwitched", &json!({"new-state": false}));
        assert!(r.changed);
        assert_eq!(s.output().1, Vec::<String>::new());
        // entering it asks OBS for the preview scene
        let r = s.event("StudioModeSwitched", &json!({"new-state": true}));
        assert!(r.refresh_preview && !r.changed);
        s.preview_scene(&json!({"name": "Break"}));
        assert_eq!(s.output().1, names(&["Break"]));
        // no preview scene while studio mode is off: nothing changes
        assert_eq!(s.preview_scene(&json!({})), Reaction::default());
    }

    #[test]
    fn changed_scenes_ask_for_the_scene_list() {
        let mut s = state("always");
        for kind in [
            "ScenesChanged", "SceneItemAdded", "SceneItemRemoved", "SceneItemVisibilityChanged",
            "SceneCollectionChanged", "SceneCollectionListChanged",
        ] {
            let r = s.event(kind, &json!({}));
            assert!(r.refresh_scenes, "{kind}");
        }
        assert_eq!(s.event("SomethingElse", &json!({})), Reaction::default());
    }

    #[test]
    fn live_modes() {
        // (mode, streaming, recording) -> is the program shown as preview?
        let cases = [
            ("always", false, false, false),
            ("always", true, true, false),
            ("stream", false, false, true),
            ("stream", true, false, false),
            ("stream", false, true, true),
            ("record", false, false, true),
            ("record", false, true, false),
            ("record", true, false, true),
            ("streamOrRecord", false, false, true),
            ("streamOrRecord", true, false, false),
            ("streamOrRecord", false, true, false),
        ];
        for (mode, streaming, recording, as_preview) in cases {
            let mut s = state(mode);
            s.scene_list(&scenes());
            s.streaming = streaming;
            s.recording = recording;
            let (programs, previews) = s.output();
            if as_preview {
                assert_eq!(
                    (programs, previews),
                    (vec![], names(&["Main", "Lower third"])),
                    "{mode} {streaming} {recording}"
                );
            } else {
                assert_eq!(programs, names(&["Main", "Lower third"]), "{mode} {streaming} {recording}");
            }
        }
    }

    #[test]
    fn stream_and_recording_events() {
        let mut s = state("streamOrRecord");
        s.scene_list(&scenes());
        assert!(s.output().0.is_empty());
        s.event("StreamStarting", &json!({}));
        assert!(!s.output().0.is_empty());
        s.event("StreamStopped", &json!({}));
        s.event("RecordingStarting", &json!({}));
        assert!(!s.output().0.is_empty());
        s.event("RecordingPaused", &json!({}));
        assert!(s.output().0.is_empty());
        s.event("RecordingResumed", &json!({}));
        assert!(!s.output().0.is_empty());
        s.event("RecordingStopped", &json!({}));
        s.event("StreamStatus", &json!({"streaming": true, "recording": false}));
        assert!(!s.output().0.is_empty());
        s.streaming_status(&json!({"streaming": false, "recording": true, "recording-paused": true}));
        assert!(s.output().0.is_empty());
    }

    // ------------------------------------------------------------------
    // the connector against a fake obs-websocket
    // ------------------------------------------------------------------

    /// Plays OBS: answers requests and, once a client has asked for the scene list, switches to "Break".
    async fn fake_obs(listener: TcpListener, auth_required: bool) {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else { continue };
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let request: Value = serde_json::from_str(&text).unwrap();
                let id = request["message-id"].clone();
                let reply = match request["request-type"].as_str().unwrap() {
                    "GetAuthRequired" => json!({"message-id": id, "status": "ok", "authRequired": auth_required}),
                    "GetSceneList" => {
                        let mut data = scenes();
                        data["message-id"] = id;
                        data["status"] = "ok".into();
                        data
                    }
                    "GetPreviewScene" => json!({"message-id": id, "status": "error", "error": "studio mode not enabled"}),
                    "GetStreamingStatus" => json!({"message-id": id, "status": "ok", "streaming": false, "recording": false}),
                    _ => json!({"message-id": id, "status": "error", "error": "unknown"}),
                };
                let was_scene_list = request["request-type"] == "GetSceneList";
                ws.send(Message::text(reply.to_string())).await.unwrap();
                if was_scene_list {
                    let event = json!({"update-type": "SwitchScenes", "scene-name": "Break"});
                    ws.send(Message::text(event.to_string())).await.unwrap();
                }
            }
        }
    }

    async fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
        for _ in 0..200 {
            if condition() {
                return;
            }
            sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {what}");
    }

    fn config(port: u16, live_mode: &str) -> ObsConfig {
        ObsConfig {
            endpoint: Endpoint { ip: "127.0.0.1".into(), port },
            live_mode: live_mode.into(),
        }
    }

    #[tokio::test]
    async fn connects_to_obs_and_reports_scenes_and_tallies() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(fake_obs(listener, false));

        let hub = Hub::new(HubOptions::default(), AppConfig::default());
        let mut obs = ObsConnector::new(Arc::clone(&hub), config(port, "always"));
        assert!(!obs.is_connected());
        obs.connect();

        wait_for("the connection", || hub.is_mixer_connected()).await;
        assert!(obs.is_connected());
        wait_for("the scenes", || hub.channels().len() == 4).await;
        assert_eq!(hub.channels()[0].name.as_deref(), Some("Main"));
        wait_for("the program", || hub.programs() == Some(names(&["Break"]))).await;
        assert_eq!(hub.previews(), Some(vec![]));

        obs.disconnect();
        assert!(!obs.is_connected());
        assert!(!hub.is_mixer_connected());
    }

    #[tokio::test]
    async fn does_not_connect_when_a_password_is_needed() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(fake_obs(listener, true));

        let hub = Hub::new(HubOptions::default(), AppConfig::default());
        let mut obs = ObsConnector::new(Arc::clone(&hub), config(port, "always"));
        obs.connect();
        sleep(Duration::from_millis(300)).await;
        assert!(!obs.is_connected());
        assert!(!hub.is_mixer_connected());
        obs.disconnect();
    }

    #[tokio::test]
    async fn reconnects_when_obs_comes_back() {
        // nobody listens first
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let hub = Hub::new(HubOptions::default(), AppConfig::default());
        let mut obs = ObsConnector::new(Arc::clone(&hub), config(port, "always"));
        obs.connect();
        sleep(Duration::from_millis(200)).await;
        assert!(!obs.is_connected());

        let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        tokio::spawn(fake_obs(listener, false));
        wait_for("the connection", || hub.is_mixer_connected()).await;
        obs.disconnect();
    }
}
