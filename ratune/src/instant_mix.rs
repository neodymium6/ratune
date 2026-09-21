//! Request bookkeeping, kept independent of the audio engine for deterministic tests.

use std::collections::HashSet;
use std::time::Instant;

use ratune_subsonic::Song;

use crate::state::QueueState;

pub const MIX_COUNT: u32 = 50;

struct PendingMix {
    id: u64,
    started: Instant,
    queue_ids: Vec<String>,
    play_gen: u64,
    paused: bool,
}

#[derive(Default)]
pub struct InstantMixState {
    next_id: u64,
    pending: Option<PendingMix>,
}

impl InstantMixState {
    pub fn started(&self) -> Option<Instant> {
        self.pending.as_ref().map(|p| p.started)
    }

    pub fn cancel(&mut self) -> bool {
        self.pending.take().is_some()
    }

    pub fn start(&mut self, queue: &QueueState, play_gen: u64, paused: bool) -> u64 {
        self.next_id += 1;
        self.pending = Some(PendingMix {
            id: self.next_id,
            started: Instant::now(),
            queue_ids: queue.songs.iter().map(|s| s.id.clone()).collect(),
            play_gen,
            paused,
        });
        self.next_id
    }

    /// Return a replacement only for the latest request and unchanged playback/queue.
    /// Cancellation and late responses are silent. Errors never mutate the queue.
    pub fn finish(
        &mut self,
        id: u64,
        queue: &QueueState,
        play_gen: u64,
        paused: bool,
        seed: Song,
        result: Result<Vec<Song>, String>,
    ) -> Result<Option<Vec<Song>>, String> {
        if self.pending.as_ref().map(|p| p.id) != Some(id) {
            return Ok(None);
        }
        let pending = self.pending.take().unwrap();
        if pending.play_gen != play_gen
            || pending.paused != paused
            || !pending
                .queue_ids
                .iter()
                .eq(queue.songs.iter().map(|s| &s.id))
        {
            return Err("Instant Mix cancelled: queue or playback changed".into());
        }
        let songs = result.map_err(|e| format!("Instant Mix failed: {e}"))?;
        let mut seen = HashSet::from([seed.id.clone()]);
        let mut mix = vec![seed];
        mix.extend(
            songs
                .into_iter()
                .filter(|s| !s.id.trim().is_empty() && seen.insert(s.id.clone()))
                .take(MIX_COUNT as usize),
        );
        if mix.len() == 1 {
            return Err("Instant Mix: no similar songs — queue unchanged".into());
        }
        Ok(Some(mix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str) -> Song {
        serde_json::from_value(serde_json::json!({"id": id, "title": id})).unwrap()
    }

    fn queue() -> QueueState {
        let mut queue = QueueState::default();
        queue.push(song("old"));
        queue
    }

    #[test]
    fn mix_starts_with_seed_and_preserves_server_order_without_duplicates() {
        let queue = queue();
        let mut state = InstantMixState::default();
        let id = state.start(&queue, 2, false);
        let mix = state
            .finish(
                id,
                &queue,
                2,
                false,
                song("seed"),
                Ok(vec![
                    song("b"),
                    song("seed"),
                    song("a"),
                    song("b"),
                    song(""),
                ]),
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            mix.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["seed", "b", "a"]
        );
        assert!(state.started().is_none());
        assert_eq!(queue.songs[0].id, "old");
    }

    #[test]
    fn errors_empty_and_seed_only_leave_queue_unchanged() {
        for response in [
            Err("unsupported".into()),
            Ok(vec![]),
            Ok(vec![song("seed")]),
        ] {
            let queue = queue();
            let mut state = InstantMixState::default();
            let id = state.start(&queue, 2, false);
            assert!(state
                .finish(id, &queue, 2, false, song("seed"), response)
                .is_err());
            assert_eq!(queue.songs.len(), 1);
            assert_eq!(queue.songs[0].id, "old");
            assert!(state.started().is_none());
        }
    }

    #[test]
    fn modified_queue_rejects_late_mix() {
        let mut queue = queue();
        let mut state = InstantMixState::default();
        let id = state.start(&queue, 2, false);
        queue.push(song("new"));
        assert!(state
            .finish(id, &queue, 2, false, song("seed"), Ok(vec![song("b")]))
            .is_err());
        assert_eq!(queue.songs.len(), 2);
    }

    #[test]
    fn changed_playback_or_pause_rejects_late_mix() {
        for (generation, paused) in [(3, false), (2, true)] {
            let queue = queue();
            let mut state = InstantMixState::default();
            let id = state.start(&queue, 2, false);
            assert!(state
                .finish(
                    id,
                    &queue,
                    generation,
                    paused,
                    song("seed"),
                    Ok(vec![song("b")])
                )
                .is_err());
        }
    }

    #[test]
    fn reordered_queue_rejects_late_mix() {
        let mut queue = queue();
        queue.push(song("other"));
        let mut state = InstantMixState::default();
        let id = state.start(&queue, 2, false);
        queue.songs.reverse();
        assert!(state
            .finish(id, &queue, 2, false, song("seed"), Ok(vec![song("b")]))
            .is_err());
    }

    #[test]
    fn highlighting_another_row_does_not_cancel_the_request() {
        let mut queue = queue();
        queue.push(song("other"));
        let mut state = InstantMixState::default();
        let id = state.start(&queue, 2, false);
        queue.cursor = 1;
        assert!(state
            .finish(id, &queue, 2, false, song("seed"), Ok(vec![song("b")]))
            .unwrap()
            .is_some());
    }

    #[test]
    fn oversized_response_is_limited_to_seed_and_requested_count() {
        let queue = queue();
        let mut state = InstantMixState::default();
        let id = state.start(&queue, 2, false);
        let songs = (0..100).map(|i| song(&i.to_string())).collect();
        let mix = state
            .finish(id, &queue, 2, false, song("seed"), Ok(songs))
            .unwrap()
            .unwrap();
        assert_eq!(mix.len(), MIX_COUNT as usize + 1);
        assert_eq!(mix[1].id, "0");
        assert_eq!(mix.last().unwrap().id, "49");
    }

    #[test]
    fn cancellation_and_old_response_do_not_consume_new_request() {
        let queue = queue();
        let mut state = InstantMixState::default();
        let old = state.start(&queue, 2, false);
        assert!(state.cancel());
        assert!(state
            .finish(old, &queue, 2, false, song("seed"), Ok(vec![song("b")]))
            .unwrap()
            .is_none());
        let new = state.start(&queue, 2, false);
        assert_ne!(old, new);
        assert!(state
            .finish(
                old,
                &queue,
                2,
                false,
                song("seed"),
                Err("late error".into())
            )
            .unwrap()
            .is_none());
        assert!(state.started().is_some());
        assert!(state
            .finish(new, &queue, 2, false, song("seed"), Ok(vec![song("b")]))
            .unwrap()
            .is_some());
    }
}
