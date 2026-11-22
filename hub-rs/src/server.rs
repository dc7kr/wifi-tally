//! The browser-facing side: serves the web frontend and speaks the socket.io
//! protocol the frontend expects.
//!
//! The frontend subscribes to topics (`events.<topic>.subscribe`). A topic is
//! a socket.io room; whenever something changes, the new state is broadcast to
//! the room.

use crate::domain::tally::TallyType;
use crate::domain::tally_config::TallyConfiguration;
use crate::events::Event;
use crate::hub::Hub;
use crate::mixer::config as mixers;
use axum::extract::ConnectInfo;
use axum::Router;
use serde_json::{json, Value};
use socketioxide::extract::{Data, SocketRef};
use socketioxide::SocketIo;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;
use tower_http::services::{ServeDir, ServeFile};
use tracing::{debug, error, warn};

const TOPICS: [&str; 6] = ["mixer", "program", "config", "tally", "channel", "tallyLog"];

const MIXERS_WITH_SETTINGS: [&str; 7] = [
    mixers::ATEM,
    mixers::MOCK,
    mixers::OBS,
    mixers::ROLAND_V8HD,
    mixers::ROLAND_V60HD,
    mixers::TEST,
    mixers::VMIX,
];

type Message = (String, String, Value);

fn msg(event: &str, payload: Value) -> (String, Value) {
    (event.to_string(), payload)
}

fn config_state_mixer(hub: &Hub) -> Value {
    json!({
        "mixerName": hub.mixer_selection().unwrap_or_default(),
        "allowedMixers": hub.allowed_mixers(),
    })
}

fn config_state(hub: &Hub, id: &str) -> Option<(String, Value)> {
    Some(msg(
        &format!("config.state.{id}"),
        hub.mixer_settings_json(id)?,
    ))
}

/// The complete current state of one topic, as sent right after subscribing.
fn snapshot(hub: &Hub, topic: &str) -> Vec<(String, Value)> {
    match topic {
        "mixer" => vec![msg(
            "mixer.state",
            json!({"isConnected": hub.is_mixer_connected()}),
        )],
        "program" => vec![msg(
            "program.state",
            json!({"programs": hub.programs(), "previews": hub.previews()}),
        )],
        "config" => {
            let mut all = Vec::new();
            // the "test" mixer has no settings page and is not announced
            all.extend(config_state(hub, mixers::ATEM));
            all.push(msg("config.state.mixer", config_state_mixer(hub)));
            for id in [
                mixers::MOCK,
                mixers::OBS,
                mixers::ROLAND_V8HD,
                mixers::ROLAND_V60HD,
                mixers::VMIX,
            ] {
                all.extend(config_state(hub, id));
            }
            all.push(msg("config.state.tallyconfig", hub.tally_defaults_json()));
            all
        }
        "tally" => vec![msg("tally.state", json!({"tallies": hub.tallies_json()}))],
        "channel" => vec![msg(
            "channel.state",
            json!({"channels": hub.channels_json()}),
        )],
        "tallyLog" => vec![msg("tally.log.state", hub.tally_logs_json())],
        _ => Vec::new(),
    }
}

/// Which room should be told what because of an event.
fn route(hub: &Hub, event: Event) -> Vec<Message> {
    let to = |room: &str, (ev, payload): (String, Value)| (room.to_string(), ev, payload);
    match event {
        Event::MixerConnected | Event::MixerDisconnected => snapshot(hub, "mixer")
            .into_iter()
            .map(|m| to("mixer", m))
            .collect(),
        Event::ProgramChanged => snapshot(hub, "program")
            .into_iter()
            .map(|m| to("program", m))
            .collect(),
        Event::TalliesChanged => snapshot(hub, "tally")
            .into_iter()
            .map(|m| to("tally", m))
            .collect(),
        Event::ChannelsChanged => snapshot(hub, "channel")
            .into_iter()
            .map(|m| to("channel", m))
            .collect(),
        Event::MixerSettingsChanged(id) => config_state(hub, &id)
            .filter(|_| MIXERS_WITH_SETTINGS.contains(&id.as_str()) && id != mixers::TEST)
            .map(|m| to("config", m))
            .into_iter()
            .collect(),
        Event::TallyDefaultsChanged => vec![to(
            "config",
            msg("config.state.tallyconfig", hub.tally_defaults_json()),
        )],
        Event::MixerSelectionChanged => vec![to(
            "config",
            msg("config.state.mixer", config_state_mixer(hub)),
        )],
        Event::TallyLogged { tally_id, log } => vec![to(
            "tallyLog",
            msg(
                "tally.log",
                json!({"tallyId": tally_id, "log": log.to_json()}),
            ),
        )],
        Event::WebTallyState {
            tally_name,
            tally,
            command,
        } => vec![(
            web_room(&tally_name),
            "webTally.state".to_string(),
            json!({"tally": tally, "command": command.as_str()}),
        )],
        Event::ConfigChanged => Vec::new(),
    }
}

fn web_room(tally_name: &str) -> String {
    format!("web:{tally_name}")
}

/// Passes events of the hub on to the browsers.
async fn forward_events(hub: Arc<Hub>, io: SocketIo) {
    let mut events = hub.subscribe();
    loop {
        let messages = match events.recv().await {
            Ok(event) => route(&hub, event),
            Err(RecvError::Lagged(missed)) => {
                warn!("missed {missed} events; resending the complete state to all browsers");
                TOPICS
                    .iter()
                    .flat_map(|topic| {
                        snapshot(&hub, topic)
                            .into_iter()
                            .map(move |(ev, payload)| (topic.to_string(), ev, payload))
                    })
                    .collect()
            }
            Err(RecvError::Closed) => break,
        };
        for (room, event, payload) in messages {
            if let Err(e) = io.to(room).emit(event, &payload).await {
                debug!("broadcast failed: {e}");
            }
        }
    }
}

/// All arguments of a socket.io event.
///
/// `Data<Value>` would only deliver the first argument, and a tuple needs a
/// fixed number of them, but several events have optional trailing arguments.
/// socketioxide hands over *all* arguments to anything that deserializes as a
/// tuple, so this asks for one of unknown size.
struct EventArgs(Vec<Value>);

impl<'de> serde::Deserialize<'de> for EventArgs {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Collect;
        impl<'de> serde::de::Visitor<'de> for Collect {
            type Value = EventArgs;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("the arguments of an event")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<EventArgs, A::Error> {
                let mut all = Vec::new();
                while let Some(arg) = seq.next_element::<Value>()? {
                    all.push(arg);
                }
                Ok(EventArgs(all))
            }
        }
        deserializer.deserialize_tuple(usize::MAX, Collect)
    }
}

type Args = Data<EventArgs>;

fn arg_str(args: &[Value], index: usize) -> Option<String> {
    args.get(index).and_then(Value::as_str).map(str::to_string)
}

fn arg_tally(args: &[Value]) -> Option<(String, TallyType)> {
    let name = arg_str(args, 0)?;
    let Some(ty) = arg_str(args, 1).as_deref().and_then(TallyType::parse) else {
        warn!("ignoring request for tally \"{name}\" without a valid tally type");
        return None;
    };
    Some((name, ty))
}

fn client_address(socket: &SocketRef) -> String {
    let parts = socket.req_parts();
    if let Some(ConnectInfo(addr)) = parts.extensions.get::<ConnectInfo<SocketAddr>>() {
        return addr.ip().to_canonical().to_string();
    }
    parts
        .headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn on_connection(hub: &Arc<Hub>, socket: SocketRef) {
    for topic in TOPICS {
        let h = Arc::clone(hub);
        socket.on(format!("events.{topic}.subscribe"), move |s: SocketRef| {
            let h = Arc::clone(&h);
            async move {
                s.join(topic);
                for (event, payload) in snapshot(&h, topic) {
                    let _ = s.emit(event, &payload);
                }
            }
        });
        socket.on(
            format!("events.{topic}.unsubscribe"),
            move |s: SocketRef| async move {
                s.leave(topic);
            },
        );
    }

    let h = Arc::clone(hub);
    socket.on(
        "events.webTally.subscribe",
        move |s: SocketRef, Data(EventArgs(a)): Args| {
            let h = Arc::clone(&h);
            async move {
                let Some(name) = arg_str(&a, 0) else { return };
                // join first: the state the hub pushes while subscribing is meant for this socket, too
                s.join(web_room(&name));
                if !h.web_subscribe(&name, &s.id.to_string(), &client_address(&s)) {
                    s.leave(web_room(&name));
                    let _ = s.emit("webTally.invalid", &name);
                }
            }
        },
    );
    let h = Arc::clone(hub);
    socket.on(
        "events.webTally.unsubscribe",
        move |s: SocketRef, Data(EventArgs(a)): Args| {
            let h = Arc::clone(&h);
            async move {
                let Some(name) = arg_str(&a, 0) else { return };
                s.leave(web_room(&name));
                h.web_unsubscribe(&name, &s.id.to_string());
            }
        },
    );
    let h = Arc::clone(hub);
    socket.on_disconnect(move |s: SocketRef| {
        let h = Arc::clone(&h);
        async move { h.web_unsubscribe_all(&s.id.to_string()) }
    });

    let h = Arc::clone(hub);
    socket.on("tally.patch", move |Data(EventArgs(a)): Args| {
        let h = Arc::clone(&h);
        async move {
            if let Some((name, ty)) = arg_tally(&a) {
                h.patch(&name, ty, arg_str(&a, 2));
            }
        }
    });
    let h = Arc::clone(hub);
    socket.on("tally.highlight", move |Data(EventArgs(a)): Args| {
        let h = Arc::clone(&h);
        async move {
            if let Some((name, ty)) = arg_tally(&a) {
                h.highlight(&name, ty);
            }
        }
    });
    let h = Arc::clone(hub);
    socket.on("tally.remove", move |Data(EventArgs(a)): Args| {
        let h = Arc::clone(&h);
        async move {
            if let Some((name, ty)) = arg_tally(&a) {
                h.remove(&name, ty);
            }
        }
    });
    let h = Arc::clone(hub);
    socket.on("tally.create", move |Data(EventArgs(a)): Args| {
        let h = Arc::clone(&h);
        async move {
            if let Some(name) = arg_str(&a, 0) {
                h.web_create(&name, arg_str(&a, 1));
            }
        }
    });
    let h = Arc::clone(hub);
    socket.on("tally.settings", move |Data(EventArgs(a)): Args| {
        let h = Arc::clone(&h);
        async move {
            if let (Some((name, ty)), Some(settings)) = (arg_tally(&a), a.get(2)) {
                h.update_settings(&name, ty, TallyConfiguration::from_json(settings));
            }
        }
    });

    for id in MIXERS_WITH_SETTINGS {
        let h = Arc::clone(hub);
        socket.on(
            format!("config.change.{id}"),
            move |Data(EventArgs(a)): Args| {
                let h = Arc::clone(&h);
                async move {
                    if let Some(settings) = a.first() {
                        h.set_mixer_settings(id, settings);
                    }
                    if let Some(mixer) = arg_str(&a, 1).filter(|m| !m.is_empty()) {
                        h.set_mixer_selection(&mixer);
                    }
                }
            },
        );
    }
    let h = Arc::clone(hub);
    socket.on("config.change.null", move |Data(EventArgs(a)): Args| {
        let h = Arc::clone(&h);
        async move {
            if let Some(mixer) = arg_str(&a, 0).filter(|m| !m.is_empty()) {
                h.set_mixer_selection(&mixer);
            }
        }
    });
    let h = Arc::clone(hub);
    socket.on(
        "config.change.tallyconfig",
        move |Data(EventArgs(a)): Args| {
            let h = Arc::clone(&h);
            async move {
                if let Some(conf) = a.first() {
                    h.set_tally_defaults(conf);
                }
            }
        },
    );
}

pub fn frontend_router(frontend_dir: Option<&Path>) -> Router {
    match frontend_dir.filter(|d| d.join("index.html").is_file()) {
        Some(dir) => {
            // fall back to the index page to allow deep-links into the application
            let service = ServeDir::new(dir).fallback(ServeFile::new(dir.join("index.html")));
            Router::new().fallback_service(service)
        }
        None => {
            if let Some(dir) = frontend_dir {
                error!("Frontend not found in {}", dir.display());
            }
            Router::new().fallback(|| async {
                "The vTally hub is running, but its web frontend was not found. \
                 Use --frontend-dir to point to the built frontend."
            })
        }
    }
}

/// Builds the web server and starts forwarding hub events to connected browsers.
pub fn build(hub: Arc<Hub>, frontend_dir: Option<PathBuf>) -> Router {
    let (layer, io) = SocketIo::new_layer();
    let h = Arc::clone(&hub);
    io.ns("/", move |socket: SocketRef| {
        let h = Arc::clone(&h);
        async move { on_connection(&h, socket) }
    });
    tokio::spawn(forward_events(hub, io));
    frontend_router(frontend_dir.as_deref()).layer(layer)
}
