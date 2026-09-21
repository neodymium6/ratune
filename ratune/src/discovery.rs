//! Discovery shelves and guarded album requests, independent of the audio engine.
use ratune_subsonic::{Album, Song};
use std::{
    collections::{hash_map::DefaultHasher, HashMap, HashSet},
    hash::{Hash, Hasher},
    time::Instant,
};

use crate::{action::Direction, history::PlayHistory, state::QueueState};

pub const SHELF_LIMIT: usize = 24;
pub const MIX_SEED_LIMIT: usize = 8;
pub const RECENT_DAYS: i64 = 14;

#[derive(Clone)]
pub struct RediscoverAlbum {
    pub album: Album,
    pub last_played: Option<i64>,
}

#[derive(Default)]
pub struct Discovery {
    /// 0 = newest, 1 = rediscover, 2 = recent Mix seeds.
    pub section: usize,
    pub selected: [usize; 3],
    pub scroll: [usize; 3],
    pub visible: [usize; 3],
    pub hits: Vec<(ratatui::layout::Rect, usize, usize)>,
    pub newest: Vec<Album>,
    pub candidates: Vec<Album>,
    pub rediscover: Vec<RediscoverAlbum>,
    pub seeds: Vec<Song>,
    pub loading: bool,
    pub error: Option<String>,
    pub refreshed: Option<Instant>,
    pub request_id: u64,
}

impl Discovery {
    pub fn len(&self, section: usize) -> usize {
        match section {
            0 => self.newest.len(),
            1 => self.rediscover.len(),
            _ => self.seeds.len(),
        }
    }

    pub fn move_selection(&mut self, direction: Direction) {
        let section = self.section;
        let max = self.len(section).saturating_sub(1);
        let pos = self.selected[section];
        let page = self.visible[section].max(1);
        self.selected[section] = match direction {
            Direction::Up => pos.saturating_sub(1),
            Direction::Down => pos.saturating_add(1).min(max),
            Direction::Top => 0,
            Direction::Bottom => max,
            Direction::PageUp => pos.saturating_sub(page),
            Direction::PageDown => pos.saturating_add(page).min(max),
        };
    }

    pub fn selected_album(&self) -> Option<&Album> {
        match self.section {
            0 => self.newest.get(self.selected[0]),
            1 => self.rediscover.get(self.selected[1]).map(|a| &a.album),
            _ => None,
        }
    }

    pub fn selected_seed(&self) -> Option<&Song> {
        (self.section == 2)
            .then(|| self.seeds.get(self.selected[2]))
            .flatten()
    }

    pub fn set_newest(&mut self, albums: Vec<Album>) {
        let selected = self.newest.get(self.selected[0]).map(|a| a.id.clone());
        self.newest = unique_albums(albums);
        if let Some(index) = self
            .newest
            .iter()
            .position(|a| Some(&a.id) == selected.as_ref())
        {
            self.selected[0] = index;
        }
    }

    pub fn rebuild_local(
        &mut self,
        history: &PlayHistory,
        index: &HashMap<String, Song>,
        mut catalog: Vec<Album>,
        now: i64,
    ) {
        let album_id = self
            .rediscover
            .get(self.selected[1])
            .map(|a| a.album.id.clone());
        let song_id = self.seeds.get(self.selected[2]).map(|s| s.id.clone());
        catalog.extend(self.candidates.iter().cloned());
        self.rediscover = rediscover(history, catalog, now, self.request_id);
        self.seeds = recent_seeds(history, index);
        if let Some(index) = self
            .rediscover
            .iter()
            .position(|a| Some(&a.album.id) == album_id.as_ref())
        {
            self.selected[1] = index;
        }
        if let Some(index) = self
            .seeds
            .iter()
            .position(|s| Some(&s.id) == song_id.as_ref())
        {
            self.selected[2] = index;
        }
        for section in 0..3 {
            self.selected[section] =
                self.selected[section].min(self.len(section).saturating_sub(1));
            self.scroll[section] = self.scroll[section].min(self.selected[section]);
        }
    }
}

pub fn unique_albums(albums: Vec<Album>) -> Vec<Album> {
    let mut seen = HashSet::new();
    albums
        .into_iter()
        .filter(|a| !a.id.is_empty() && seen.insert(a.id.clone()))
        .take(SHELF_LIMIT)
        .collect()
}

pub fn rediscover(
    history: &PlayHistory,
    albums: Vec<Album>,
    now: i64,
    refresh: u64,
) -> Vec<RediscoverAlbum> {
    let mut last = HashMap::<&str, i64>::new();
    for record in &history.records {
        let time = last.entry(&record.album_id).or_insert(record.played_at);
        *time = (*time).max(record.played_at);
    }
    let cutoff = now - RECENT_DAYS * 86400;
    let mut seen = HashSet::new();
    let mut candidates: Vec<_> = albums
        .into_iter()
        .filter(|a| !a.id.is_empty() && seen.insert(a.id.clone()))
        .filter_map(|album| {
            let last_played = last.get(album.id.as_str()).copied();
            (last_played.is_none_or(|t| t < cutoff))
                .then_some(RediscoverAlbum { album, last_played })
        })
        .collect();
    // Known old favorites first, oldest first; never-played-here is an explicit fallback.
    // Vary ties only on refresh, not whenever the user returns to Home. Catalog
    // iteration order is arbitrary, so hashing also stabilizes the selected set.
    candidates.sort_by_key(|a| {
        let mut hasher = DefaultHasher::new();
        (refresh, &a.album.id).hash(&mut hasher);
        (
            a.last_played.is_none(),
            a.last_played.unwrap_or(0),
            hasher.finish(),
        )
    });
    candidates.truncate(SHELF_LIMIT);
    candidates
}

pub fn recent_seeds(history: &PlayHistory, index: &HashMap<String, Song>) -> Vec<Song> {
    let mut seen = HashSet::new();
    history.records.iter().rev().filter(|r| !r.song_id.is_empty() && seen.insert(r.song_id.clone()))
        .take(MIX_SEED_LIMIT).map(|record| {
            index.get(&record.song_id).cloned().unwrap_or_else(|| {
                // Preserve artwork and identity even if the metadata index is disabled.
                serde_json::from_value(serde_json::json!({
                    "id": record.song_id, "title": record.track_name,
                    "albumId": record.album_id, "album": record.album_name,
                    "artistId": record.artist_id, "artist": record.artist_name,
                    "coverArt": record.album_id, "duration": record.duration_secs.min(u32::MAX as u64),
                })).expect("valid history song")
            })
        }).collect()
}

#[derive(Default)]
pub struct AlbumRequest {
    next: u64,
    pending: Option<(u64, bool, Vec<String>, u64, bool)>,
}

impl AlbumRequest {
    pub fn cancel(&mut self) -> bool {
        self.pending.take().is_some()
    }

    pub fn start(
        &mut self,
        replace: bool,
        queue: &QueueState,
        generation: u64,
        paused: bool,
    ) -> u64 {
        self.next += 1;
        self.pending = Some((
            self.next,
            replace,
            queue.songs.iter().map(|s| s.id.clone()).collect(),
            generation,
            paused,
        ));
        self.next
    }

    pub fn finish(
        &mut self,
        id: u64,
        queue: &QueueState,
        generation: u64,
        paused: bool,
        result: Result<Vec<Song>, String>,
    ) -> Result<Option<(bool, Vec<Song>)>, String> {
        if self.pending.as_ref().map(|p| p.0) != Some(id) {
            return Ok(None);
        }
        let (_, replace, ids, old_generation, old_paused) = self.pending.take().unwrap();
        if old_generation != generation
            || old_paused != paused
            || !ids.iter().eq(queue.songs.iter().map(|s| &s.id))
        {
            return Err("Album action cancelled: queue or playback changed".into());
        }
        let mut songs = result?;
        let mut seen = HashSet::new();
        songs.retain(|s| !s.id.is_empty() && seen.insert(s.id.clone()));
        songs.sort_by_key(|s| (s.disc_number.unwrap_or(1), s.track.unwrap_or(0)));
        if songs.is_empty() {
            return Err("Album has no tracks — queue unchanged".into());
        }
        Ok(Some((replace, songs)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn album(id: &str) -> Album {
        serde_json::from_value(serde_json::json!({"id":id,"name":id})).unwrap()
    }
    fn song(id: &str) -> Song {
        serde_json::from_value(serde_json::json!({"id":id,"title":id})).unwrap()
    }
    fn history() -> PlayHistory {
        let mut history = PlayHistory::default();
        for (id, at) in [("old", 1), ("recent", 30 * 86400), ("recent", 31 * 86400)] {
            history.records.push(crate::history::PlayRecord {
                song_id: id.into(),
                album_id: id.into(),
                artist_id: "artist".into(),
                artist_name: "Artist".into(),
                album_name: id.into(),
                track_name: id.into(),
                played_at: at,
                duration_secs: 60,
            });
        }
        history
    }
    #[test]
    fn rediscover_excludes_recent_and_deduplicates_with_explicit_unplayed_fallback() {
        let result = rediscover(
            &history(),
            vec![album("recent"), album("old"), album("new"), album("old")],
            32 * 86400,
            1,
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].album.id, "old");
        assert_eq!(result[0].last_played, Some(1));
        assert_eq!(result[1].last_played, None);
    }
    #[test]
    fn mix_seeds_are_recent_unique_tracks_and_retain_art() {
        let result = recent_seeds(&history(), &HashMap::new());
        assert_eq!(
            result.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["recent", "old"]
        );
        assert_eq!(result[0].cover_art.as_deref(), Some("recent"));
    }
    #[test]
    fn home_seed_is_selected_item_not_current_playback() {
        let mut state = Discovery {
            seeds: vec![song("one"), song("two")],
            section: 2,
            ..Default::default()
        };
        state.move_selection(Direction::Down);
        assert_eq!(state.selected_seed().unwrap().id, "two");
        state.section = 0;
        assert!(state.selected_seed().is_none());
    }
    #[test]
    fn returning_home_preserves_order_and_selection_when_metadata_arrives() {
        let mut state = Discovery::default();
        let mut history = history();
        let catalog: Vec<_> = (0..40).map(|i| album(&i.to_string())).collect();
        state.rebuild_local(&history, &HashMap::new(), catalog.clone(), 32 * 86400);
        let before: Vec<_> = state
            .rediscover
            .iter()
            .map(|a| a.album.id.clone())
            .collect();
        state.selected[1] = 3;
        state.selected[2] = 1;
        let mut record = history.records[0].clone();
        record.song_id = "new seed".into();
        record.duration_secs = u64::MAX;
        history.records.push(record);
        state.rebuild_local(
            &history,
            &HashMap::new(),
            catalog.into_iter().rev().collect(),
            32 * 86400,
        );
        assert_eq!(
            before,
            state
                .rediscover
                .iter()
                .map(|a| a.album.id.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(state.selected[1], 3);
        assert_eq!(state.seeds[state.selected[2]].id, "old");
        assert_eq!(state.seeds[0].duration, Some(u32::MAX));
    }
    #[test]
    fn new_additions_preserve_server_order_and_selected_identity() {
        let mut state = Discovery::default();
        state.set_newest(vec![album("b"), album("a"), album("b")]);
        assert_eq!(state.newest.len(), 2);
        assert_eq!(state.newest[0].id, "b");
        state.selected[0] = 1;
        state.set_newest(vec![album("c"), album("b"), album("a")]);
        assert_eq!(state.newest[state.selected[0]].id, "a");
    }
    #[test]
    fn album_failure_empty_and_stale_responses_do_not_replace_queue() {
        let mut queue = QueueState::default();
        queue.push(song("playing"));
        let mut state = AlbumRequest::default();
        for result in [Err("network error".into()), Ok(Vec::new())] {
            let id = state.start(true, &queue, 1, false);
            assert!(state.finish(id, &queue, 1, false, result).is_err());
            assert_eq!(queue.songs[0].id, "playing");
        }
        let old = state.start(true, &queue, 1, false);
        let new = state.start(false, &queue, 1, false);
        assert!(state
            .finish(old, &queue, 1, false, Ok(vec![song("old")]))
            .unwrap()
            .is_none());
        queue.push(song("edited"));
        assert!(state
            .finish(new, &queue, 1, false, Ok(vec![song("new")]))
            .is_err());
    }
    #[test]
    fn album_cancel_and_playback_change_reject_late_responses() {
        let queue = QueueState::default();
        let mut state = AlbumRequest::default();
        let id = state.start(true, &queue, 1, false);
        state.cancel();
        assert!(state
            .finish(id, &queue, 1, false, Ok(vec![song("late")]))
            .unwrap()
            .is_none());
        let id = state.start(true, &queue, 1, false);
        assert!(state
            .finish(id, &queue, 2, false, Ok(vec![song("late")]))
            .is_err());
    }
}
