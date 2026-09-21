//! Typed Subsonic Jukebox protocol. Connecting uses Get only; never starts audio.
use crate::{error::SubsonicError, models::Song};
use anyhow::{bail, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JukeboxStatus {
    pub current_index: i64,
    pub playing: bool,
    pub gain: f64,
    #[serde(default)]
    pub position: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JukeboxPlaylist {
    #[serde(flatten)]
    pub status: JukeboxStatus,
    #[serde(default)]
    pub entry: Vec<Song>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JukeboxCommand {
    Get,
    Start,
    Stop,
    Clear,
    Shuffle,
    Skip { index: usize, offset: u64 },
    Remove(usize),
    Add(Vec<String>),
    Set(Vec<String>),
    Gain(f64),
}

impl JukeboxCommand {
    pub(crate) fn params(&self) -> Result<Vec<(String, String)>> {
        let mut params = Vec::new();
        let action = match self {
            Self::Get => "get",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Clear => "clear",
            Self::Shuffle => "shuffle",
            Self::Skip { index, offset } => {
                params.push(("index".into(), index.to_string()));
                params.push(("offset".into(), offset.to_string()));
                "skip"
            }
            Self::Remove(index) => {
                params.push(("index".into(), index.to_string()));
                "remove"
            }
            Self::Add(ids) | Self::Set(ids) => {
                if ids.is_empty() || ids.len() > 200 || ids.iter().any(|id| id.trim().is_empty()) {
                    bail!("Jukebox requires 1–200 valid song IDs per operation");
                }
                params.extend(ids.iter().map(|id| ("id".into(), id.clone())));
                if matches!(self, Self::Add(_)) {
                    "add"
                } else {
                    "set"
                }
            }
            Self::Gain(gain) => {
                if !gain.is_finite() || !(0.0..=1.0).contains(gain) {
                    bail!("Invalid Jukebox gain");
                }
                params.push(("gain".into(), gain.to_string()));
                "setGain"
            }
        };
        params.push(("action".into(), action.into()));
        Ok(params)
    }
}

#[derive(Deserialize)]
pub(crate) struct Envelope {
    #[serde(rename = "subsonic-response")]
    pub response: Body,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Body {
    pub status: String,
    pub error: Option<SubsonicError>,
    pub jukebox_status: Option<JukeboxStatus>,
    pub jukebox_playlist: Option<JukeboxPlaylist>,
}

#[derive(Debug)]
pub enum JukeboxResponse {
    Playlist(JukeboxPlaylist),
    Status(JukeboxStatus),
}
