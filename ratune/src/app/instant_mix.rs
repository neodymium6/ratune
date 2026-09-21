use super::*;

impl App {
    pub fn instant_mix_status(&self) -> Option<String> {
        self.instant_mix
            .started()
            .map(|started| {
                let key = crate::keybinds::format_optional(&self.keybinds.instant_mix);
                format!(
                    "Instant Mix: fetching similar songs · {}s · {key} cancel",
                    started.elapsed().as_secs()
                )
            })
            .or_else(|| {
                // Mix results must remain visible even during search or index refresh.
                self.status_flash
                    .as_ref()
                    .filter(|(message, _)| message.starts_with("Instant Mix"))
                    .map(|(message, _)| message.clone())
            })
    }

    pub(super) fn instant_mix_seed(&self) -> Option<ratune_subsonic::Song> {
        match self.active_tab {
            Tab::Home if self.config.home_discovery => self.discovery.selected_seed().cloned(),
            Tab::Browser if self.browser_focus == BrowserColumn::Tracks => {
                if self.browse_files() {
                    self.folders
                        .current_preview_track(self.browser_column_filter(BrowserColumn::Tracks))
                        .cloned()
                } else {
                    self.library.current_track().cloned()
                }
            }
            Tab::Browser => None,
            Tab::NowPlaying => self.queue.current().cloned().or_else(|| {
                (!self.radio_now_playing_active())
                    .then(|| self.playback.current_song.clone())
                    .flatten()
            }),
            Tab::Home if !self.radio_now_playing_active() => self.playback.current_song.clone(),
            Tab::Home => None,
        }
    }

    pub(super) fn handle_instant_mix(&mut self) {
        self.cancel_discovery_album();
        if self.instant_mix.cancel() {
            if let Some(task) = self.instant_mix_task.take() {
                task.abort();
            }
            self.flash_status("Instant Mix cancelled — queue unchanged");
            return;
        }
        if !self.remote_available() {
            self.flash_status("Instant Mix requires an online server");
            return;
        }
        let Some(seed) = self.instant_mix_seed().filter(|s| !s.id.trim().is_empty()) else {
            self.flash_status(
                if self.active_tab == Tab::Home && self.config.home_discovery {
                    "Instant Mix: use J/K to select Start a Mix, then choose a track"
                } else {
                    "Instant Mix: select a song in Browse or Now Playing"
                },
            );
            return;
        };
        let request_id = self
            .instant_mix
            .start(&self.queue, self.play_gen, self.playback.paused);
        let client = Arc::clone(&self.subsonic);
        let tx = self.library_tx.clone();
        self.instant_mix_task = Some(
            tokio::spawn(async move {
                let result = client
                    .get_similar_songs2(&seed.id, crate::instant_mix::MIX_COUNT)
                    .await
                    .map_err(|e| e.to_string());
                let _ = tx
                    .send(LibraryUpdate::InstantMix {
                        request_id,
                        seed,
                        result,
                    })
                    .await;
            })
            .abort_handle(),
        );
    }

    pub(super) fn apply_instant_mix(
        &mut self,
        request_id: u64,
        seed: ratune_subsonic::Song,
        result: Result<Vec<ratune_subsonic::Song>, String>,
    ) {
        let outcome = self.instant_mix.finish(
            request_id,
            &self.queue,
            self.play_gen,
            self.playback.paused,
            seed,
            result,
        );
        match outcome {
            Ok(Some(songs)) => {
                let n = songs.len();
                self.handle_clear_queue();
                self.queue.songs = songs;
                self.queue.adopt_current_order_as_shuffle_baseline();
                self.active_tab = Tab::NowPlaying;
                self.play_current();
                self.flash_status_secs(
                    format!("Instant Mix: playing {n} tracks (seed + similar songs)"),
                    8,
                );
            }
            Ok(None) => {}
            Err(message) => self.flash_status_secs(message, 8),
        }
        if self.instant_mix.started().is_none() {
            self.instant_mix_task = None;
        }
    }
}
