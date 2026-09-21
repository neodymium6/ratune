//! Remote playback state and serialized operations; independent of TUI/audio.
use crate::state::{PlaybackState, QueueState};
use ratune_subsonic::{
    JukeboxCommand as Command, JukeboxPlaylist, JukeboxResponse, Song, SubsonicClient,
};
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

pub struct LocalBackup {
    pub queue: QueueState,
    pub playback: PlaybackState,
    pub volume: u8,
    pub visualizer: bool,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RequestKind {
    Connect,
    Poll,
    Control,
}
#[derive(Default)]
pub struct Jukebox {
    pub local: Option<LocalBackup>,
    pub snapshot: Option<JukeboxPlaylist>,
    pub pending: Option<(u64, RequestKind)>,
    pub task: Option<tokio::task::AbortHandle>,
    pub next: u64,
    pub last_poll: Option<Instant>,
    pub stale: bool,
}
impl Jukebox {
    pub fn active(&self) -> bool {
        self.local.is_some()
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some_and(|(_, k)| k != RequestKind::Poll)
    }
    pub fn begin(&mut self, kind: RequestKind) -> u64 {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.next += 1;
        self.pending = Some((self.next, kind));
        self.next
    }
    pub fn finish(&mut self, id: u64) -> Option<RequestKind> {
        if self.pending.map(|p| p.0) != Some(id) {
            return None;
        }
        self.task = None;
        self.last_poll = Some(Instant::now());
        self.pending.take().map(|p| p.1)
    }
    pub fn disconnect(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.pending = None;
        self.snapshot = None;
        self.stale = false;
    }
}

#[derive(Debug, Clone)]
pub enum Intent {
    Pause,
    Play(usize),
    Next,
    Previous,
    Seek(Duration),
    Gain(f64),
    Clear,
    Remove(usize),
    Shuffle,
    Songs { songs: Vec<Song>, replace: bool },
    Album { id: String, replace: bool },
    Mix(Box<Song>),
}

pub async fn read(client: &SubsonicClient) -> Result<JukeboxPlaylist, String> {
    match client
        .jukebox_control(&Command::Get)
        .await
        .map_err(safe_error)?
    {
        JukeboxResponse::Playlist(p) => {
            if !p.status.gain.is_finite()
                || !(0.0..=1.0).contains(&p.status.gain)
                || !p.status.position.is_finite()
                || p.status.position < 0.0
                || p.status.current_index < -1
                || (p.status.current_index >= 0
                    && !p.entry.is_empty()
                    && p.status.current_index as usize >= p.entry.len())
                || (p.status.current_index == -1 && p.status.playing)
            {
                return Err("Invalid Jukebox status from server".into());
            }
            Ok(p)
        }
        _ => Err("Missing Jukebox playlist".into()),
    }
}
fn safe_error(error: anyhow::Error) -> String {
    if let Some(e) = error.downcast_ref::<ratune_subsonic::SubsonicError>() {
        format!(
            "Jukebox unavailable (server error {}): check Jukebox enablement and user permissions",
            e.code
        )
    } else {
        "Jukebox request failed: connection or invalid server response".into()
    }
}
pub fn same_queue(a: &JukeboxPlaylist, b: &JukeboxPlaylist) -> bool {
    a.entry
        .iter()
        .map(|s| &s.id)
        .eq(b.entry.iter().map(|s| &s.id))
}

/// Preserve the selected occurrence of repeated song IDs across server polls.
pub fn selection_after_refresh(old: &[Song], new: &[Song], cursor: usize) -> usize {
    old.get(cursor)
        .and_then(|selected| {
            let occurrence = old[..cursor].iter().filter(|s| s.id == selected.id).count();
            new.iter()
                .enumerate()
                .filter(|(_, s)| s.id == selected.id)
                .nth(occurrence)
                .map(|(i, _)| i)
        })
        .unwrap_or(cursor)
        .min(new.len().saturating_sub(1))
}

pub fn plan(current: &JukeboxPlaylist, intent: Intent) -> Result<Vec<Command>, String> {
    let len = current.entry.len();
    let index = usize::try_from(current.status.current_index).unwrap_or(0);
    let require_track = || {
        if len == 0 {
            Err("Jukebox queue is empty".to_string())
        } else {
            Ok(())
        }
    };
    let play = |target| {
        if target < len {
            Ok(vec![
                Command::Skip {
                    index: target,
                    offset: 0,
                },
                Command::Start,
            ])
        } else {
            Err("No track at that Jukebox queue position".into())
        }
    };
    match intent {
        Intent::Pause => {
            require_track()?;
            if current.status.current_index < 0 {
                return play(0);
            }
            Ok(vec![if current.status.playing {
                Command::Stop
            } else {
                Command::Start
            }])
        }
        Intent::Play(target) => play(target),
        Intent::Next => {
            require_track()?;
            play(if current.status.current_index < 0 {
                0
            } else {
                index + 1
            })
        }
        Intent::Previous => {
            require_track()?;
            play(index.saturating_sub(1))
        }
        Intent::Seek(position) => {
            require_track()?;
            if current.status.current_index < 0 {
                return Err("Jukebox: select a track before seeking".into());
            }
            let mut commands = vec![Command::Skip {
                index,
                offset: position.as_secs(),
            }];
            if !current.status.playing {
                commands.push(Command::Stop);
            }
            Ok(commands)
        }
        Intent::Gain(gain) if gain.is_finite() => Ok(vec![Command::Gain(gain.clamp(0.0, 1.0))]),
        Intent::Clear => Ok(vec![Command::Clear]),
        Intent::Remove(target) if target < len => Ok(vec![Command::Remove(target)]),
        Intent::Shuffle => {
            require_track()?;
            Ok(vec![Command::Shuffle])
        }
        Intent::Songs { songs, replace } => {
            let ids: Vec<_> = songs.into_iter().map(|s| s.id).collect();
            if ids.is_empty() || ids.len() > 200 || ids.iter().any(|id| id.trim().is_empty()) {
                return Err("Jukebox: choose 1–200 tracks; queue unchanged".into());
            }
            if replace {
                Ok(vec![
                    Command::Set(ids),
                    Command::Skip {
                        index: 0,
                        offset: 0,
                    },
                    Command::Start,
                ])
            } else if len == 0 {
                Ok(vec![
                    Command::Add(ids),
                    Command::Skip {
                        index: 0,
                        offset: 0,
                    },
                    Command::Start,
                ])
            } else {
                Ok(vec![Command::Add(ids)])
            }
        }
        _ => Err("Unsupported Jukebox operation".into()),
    }
}

pub async fn execute(
    client: &SubsonicClient,
    expected: &JukeboxPlaylist,
    intent: Intent,
) -> Result<JukeboxPlaylist, String> {
    let intent = match intent {
        Intent::Album { id, replace } => {
            let mut songs = client
                .get_album(&id)
                .await
                .map_err(|_| "Cannot load album; Jukebox unchanged")?
                .song;
            songs.sort_by_key(|s| (s.disc_number.unwrap_or(1), s.track.unwrap_or(0)));
            Intent::Songs { songs, replace }
        }
        Intent::Mix(seed) => {
            let seed = *seed;
            let similar = client
                .get_similar_songs2(&seed.id, crate::instant_mix::MIX_COUNT)
                .await
                .map_err(|_| "Cannot create Instant Mix; Jukebox unchanged")?;
            let mut seen = HashSet::from([seed.id.clone()]);
            let mut songs = vec![seed];
            songs.extend(
                similar
                    .into_iter()
                    .filter(|s| !s.id.trim().is_empty() && seen.insert(s.id.clone()))
                    .take(50),
            );
            if songs.len() == 1 {
                return Err("Instant Mix: no similar songs; Jukebox unchanged".into());
            }
            Intent::Songs {
                songs,
                replace: true,
            }
        }
        other => other,
    };
    let current = read(client).await?;
    if !same_queue(expected, &current) {
        return Err("Jukebox queue changed elsewhere; refreshed without overwriting it".into());
    }
    if matches!(intent, Intent::Seek(_))
        && expected.status.current_index != current.status.current_index
    {
        return Err("Jukebox track changed; seek cancelled".into());
    }
    for command in plan(&current, intent)? {
        // Deliberately no retry of mutating requests: a timeout can mean the
        // server applied the change. Re-read instead of duplicating the action.
        client.jukebox_control(&command).await.map_err(safe_error)?;
    }
    read(client).await
}

#[cfg(test)]
mod transport_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_intents_keep_channel_messages_compact() {
        assert!(std::mem::size_of::<Intent>() <= 64);
    }
    fn playlist() -> JukeboxPlaylist {
        serde_json::from_value(serde_json::json!({"currentIndex":0,"playing":true,"gain":0.5,"position":12,"entry":[{"id":"one","title":"One"},{"id":"two","title":"Two"}]})).unwrap()
    }
    #[test]
    fn append_does_not_interrupt_playback_and_replace_is_explicit() {
        let p = playlist();
        let songs = p.entry.clone();
        assert_eq!(
            plan(
                &p,
                Intent::Songs {
                    songs: songs.clone(),
                    replace: false
                }
            )
            .unwrap(),
            vec![Command::Add(vec!["one".into(), "two".into()])]
        );
        assert_eq!(
            plan(
                &p,
                Intent::Songs {
                    songs,
                    replace: true
                }
            )
            .unwrap()
            .len(),
            3
        );
        assert!(plan(
            &p,
            Intent::Songs {
                songs: vec![],
                replace: true
            }
        )
        .is_err());
    }
    #[test]
    fn next_uses_server_playing_index_and_paused_seek_stays_paused() {
        let mut p = playlist();
        assert_eq!(
            plan(&p, Intent::Next).unwrap()[0],
            Command::Skip {
                index: 1,
                offset: 0
            }
        );
        p.status.playing = false;
        assert_eq!(
            plan(&p, Intent::Seek(Duration::from_secs(40))).unwrap(),
            vec![
                Command::Skip {
                    index: 0,
                    offset: 40
                },
                Command::Stop
            ]
        );
        p.status.current_index = 1;
        assert!(plan(&p, Intent::Next).is_err());
    }
    #[test]
    fn stale_responses_and_disconnected_requests_are_ignored() {
        let mut state = Jukebox::default();
        let old = state.begin(RequestKind::Poll);
        let new = state.begin(RequestKind::Control);
        assert!(state.finish(old).is_none());
        assert!(state.busy());
        assert_eq!(state.finish(new), Some(RequestKind::Control));
        let id = state.begin(RequestKind::Connect);
        state.disconnect();
        assert!(state.finish(id).is_none());
    }

    #[test]
    fn queued_but_unstarted_playback_begins_at_first_track() {
        let mut p = playlist();
        p.status.current_index = -1;
        p.status.playing = false;
        for intent in [Intent::Pause, Intent::Next, Intent::Previous] {
            assert_eq!(
                plan(&p, intent).unwrap(),
                vec![
                    Command::Skip {
                        index: 0,
                        offset: 0
                    },
                    Command::Start
                ]
            );
        }
        assert!(plan(&p, Intent::Seek(Duration::from_secs(5))).is_err());
    }

    #[test]
    fn polling_preserves_duplicate_selection_and_clamps_removed_rows() {
        let p = playlist();
        let old = vec![p.entry[0].clone(), p.entry[1].clone(), p.entry[0].clone()];
        assert_eq!(selection_after_refresh(&old, &old, 2), 2);
        let new = vec![
            p.entry[1].clone(),
            p.entry[0].clone(),
            p.entry[1].clone(),
            p.entry[0].clone(),
        ];
        assert_eq!(selection_after_refresh(&old, &new, 2), 3);
        assert_eq!(selection_after_refresh(&old, &[], 2), 0);
    }
}
