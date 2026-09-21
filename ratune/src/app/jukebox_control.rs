use super::*;
use crate::jukebox::{self, Intent, LocalBackup, RequestKind};
use ratune_subsonic::JukeboxPlaylist;

impl App {
    pub fn jukebox_label(&self) -> &'static str {
        if self.jukebox.active() {
            if self.jukebox.stale {
                "Jukebox (stale)"
            } else if self.jukebox.busy() {
                "Jukebox (working)"
            } else {
                "Jukebox"
            }
        } else if self.jukebox.busy() {
            "Local (connecting)"
        } else {
            "Local"
        }
    }

    pub fn tick_jukebox(&mut self) {
        if self.jukebox.active()
            && self.jukebox.pending.is_none()
            && self
                .jukebox
                .last_poll
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(2))
        {
            self.jukebox_read(RequestKind::Poll);
        }
    }

    fn jukebox_read(&mut self, kind: RequestKind) {
        let id = self.jukebox.begin(kind);
        let client = self.subsonic.clone();
        let tx = self.library_tx.clone();
        self.jukebox.task = Some(
            tokio::spawn(async move {
                let result = jukebox::read(&client).await;
                let _ = tx
                    .send(LibraryUpdate::Jukebox {
                        id,
                        result,
                        recovered: None,
                    })
                    .await;
            })
            .abort_handle(),
        );
    }

    fn toggle_jukebox(&mut self) {
        if self
            .jukebox
            .pending
            .is_some_and(|(_, kind)| kind == RequestKind::Control)
        {
            self.flash_status("Jukebox operation in progress; wait before disconnecting");
            return;
        }
        if let Some(mut local) = self.jukebox.local.take() {
            self.jukebox.disconnect();
            self.player_tx.set_remote(false);
            self.queue = local.queue;
            local.playback.paused = true;
            local.playback.player_loaded = false;
            local.playback.elapsed = Duration::ZERO;
            self.playback = local.playback;
            self.config.default_volume = local.volume;
            self.visualizer_visible = local.visualizer;
            let _ = self
                .player_tx
                .send(PlayerCommand::SetVolume(local.volume as f32 / 100.0));
            self.clear_now_playing_art_cache();
            if let Some(id) = self.expected_cover_art_id().map(str::to_string) {
                self.fetch_cover_art(id);
            }
            self.play_gen += 1;
            self.flash_status_secs(
                "Local output restored (paused); server playback continues",
                6,
            );
        } else if self.jukebox.pending.is_some() {
            self.jukebox.disconnect();
            self.flash_status("Jukebox connection cancelled; local output unchanged");
        } else if !self.remote_available() {
            self.flash_status("Jukebox requires an online server");
        } else {
            self.flash_status_secs(
                "Connecting to Jukebox (read-only); no server playback changes",
                6,
            );
            self.jukebox_read(RequestKind::Connect);
        }
    }

    pub fn jukebox_operation(&mut self, intent: Intent) {
        if !self.jukebox.active() {
            return;
        }
        if self.jukebox.busy() {
            self.flash_status("Jukebox operation in progress; wait for server confirmation");
            return;
        }
        if self.jukebox.stale {
            self.flash_status("Jukebox state is stale; waiting for a successful refresh");
            return;
        }
        let Some(expected) = self.jukebox.snapshot.clone() else {
            return;
        };
        let id = self.jukebox.begin(RequestKind::Control);
        let client = self.subsonic.clone();
        let tx = self.library_tx.clone();
        self.flash_status_secs("Jukebox: waiting for server confirmation…", 25);
        self.jukebox.task = Some(
            tokio::spawn(async move {
                let result = tokio::time::timeout(
                    Duration::from_secs(20),
                    jukebox::execute(&client, &expected, intent),
                )
                .await
                .unwrap_or_else(|_| {
                    Err("Jukebox operation timed out; refreshing, not retrying".into())
                });
                let recovered = if result.is_err() {
                    jukebox::read(&client).await.ok()
                } else {
                    None
                };
                let _ = tx
                    .send(LibraryUpdate::Jukebox {
                        id,
                        result,
                        recovered,
                    })
                    .await;
            })
            .abort_handle(),
        );
    }

    pub(super) fn apply_jukebox(
        &mut self,
        id: u64,
        result: Result<JukeboxPlaylist, String>,
        recovered: Option<JukeboxPlaylist>,
    ) {
        let Some(kind) = self.jukebox.finish(id) else {
            return;
        };
        match result {
            Ok(snapshot) => {
                if kind == RequestKind::Connect {
                    self.cancel_discovery_album();
                    self.instant_mix.cancel();
                    if let Some(task) = self.instant_mix_task.take() {
                        task.abort();
                    }
                    self.pending_global_confirm = None;
                    self.player_tx.set_remote(true);
                    self.prefetch_gen.fetch_add(1, Ordering::Release);
                    self.jukebox.local = Some(LocalBackup {
                        queue: std::mem::take(&mut self.queue),
                        playback: std::mem::take(&mut self.playback),
                        volume: self.config.default_volume,
                        visualizer: self.visualizer_visible,
                    });
                    self.visualizer_visible = false;
                    self.np_pane_focus = NowPlayingPaneFocus::Queue;
                    self.active_tab = Tab::NowPlaying;
                }
                self.sync_jukebox(snapshot);
                if kind != RequestKind::Poll {
                    self.flash_status_secs(if kind == RequestKind::Connect {
                        "Jukebox connected; existing server queue adopted without changing playback"
                    } else { "Jukebox: server confirmed" }, 5);
                }
            }
            Err(error) => {
                if let Some(snapshot) = recovered {
                    self.sync_jukebox(snapshot);
                } else if self.jukebox.active() {
                    self.jukebox.stale = true;
                }
                self.flash_status_secs(error, 8);
            }
        }
    }

    fn sync_jukebox(&mut self, snapshot: JukeboxPlaylist) {
        let initial = self.jukebox.snapshot.is_none();
        let old_song = self.playback.current_song.as_ref().map(|s| s.id.clone());
        let current = usize::try_from(snapshot.status.current_index).ok();
        self.queue.loop_enabled = false;
        self.queue.cursor = if initial {
            current.unwrap_or(0)
        } else {
            jukebox::selection_after_refresh(&self.queue.songs, &snapshot.entry, self.queue.cursor)
        }
        .min(snapshot.entry.len().saturating_sub(1));
        self.queue.songs = snapshot.entry.clone();
        self.queue.scroll = self.queue.scroll.min(self.queue.cursor);
        self.playback.current_song = current.and_then(|i| snapshot.entry.get(i)).cloned();
        self.playback.total = self
            .playback
            .current_song
            .as_ref()
            .and_then(|s| s.duration)
            .map(|d| Duration::from_secs(d as u64));
        self.playback.elapsed =
            Duration::from_secs_f64(snapshot.status.position.min(86400.0 * 365.0));
        self.playback.paused = !snapshot.status.playing;
        self.playback.player_loaded = self.playback.current_song.is_some();
        self.config.default_volume = (snapshot.status.gain * 100.0).round() as u8;
        if old_song != self.playback.current_song.as_ref().map(|s| s.id.clone()) {
            self.play_gen += 1;
            self.clear_now_playing_art_cache();
            if let Some(id) = self.expected_cover_art_id().map(str::to_string) {
                self.fetch_cover_art(id);
            }
            self.lyrics_cache = None;
            self.lyrics_loading = false;
            self.lyrics_scroll = 0;
            if let Some(song) = self.playback.current_song.clone() {
                self.fetch_lyrics(
                    song.id,
                    song.artist.unwrap_or_default(),
                    song.title,
                    song.album.unwrap_or_default(),
                );
            }
        }
        self.jukebox.snapshot = Some(snapshot);
        self.jukebox.stale = false;
    }

    fn selected_jukebox_album(&self) -> Option<String> {
        if self.active_tab == Tab::Home && self.config.home_discovery {
            self.discovery.selected_album().map(|a| a.id.clone())
        } else if self.active_tab == Tab::Browser {
            self.library.current_album().map(|a| a.id.clone())
        } else {
            None
        }
    }
    fn jukebox_add_selection(&mut self, replace: bool, album_only: bool) {
        if self.active_tab == Tab::Browser && self.browser_browse_mode == BrowseMode::Files {
            self.flash_status("Jukebox: use artist/album browsing for this first version");
            return;
        }
        let song = if !album_only && self.active_tab == Tab::Home {
            self.discovery.selected_seed().cloned()
        } else if !album_only
            && self.active_tab == Tab::Browser
            && self.browser_focus == BrowserColumn::Tracks
        {
            self.library.current_track().cloned()
        } else {
            None
        };
        if let Some(song) = song {
            self.jukebox_operation(Intent::Songs {
                songs: vec![song],
                replace,
            });
        } else if let Some(id) = self.selected_jukebox_album() {
            self.jukebox_operation(Intent::Album { id, replace });
        } else {
            self.flash_status("Jukebox: select an album or track in Home/Browse");
        }
    }

    /// Consume all remote playback actions here. Unknown actions are explicitly
    /// unavailable, never silently handed to a local audio/queue mutation path.
    pub(super) fn dispatch_jukebox(&mut self, action: &Action) -> bool {
        use Action::*;
        if matches!(action, ToggleJukebox) {
            self.toggle_jukebox();
            return true;
        }
        if !self.jukebox.active() {
            return false;
        }
        if matches!(action, Quit) && self.jukebox.busy() {
            self.flash_status("Jukebox operation in progress; wait before quitting");
            return true;
        }
        match action {
            PlayPause => self.jukebox_operation(Intent::Pause),
            NextTrack => self.jukebox_operation(Intent::Next),
            PrevTrack => self.jukebox_operation(Intent::Previous),
            VolumeUp => self.jukebox_operation(Intent::Gain(
                (self.config.default_volume as f64 + 5.0) / 100.0,
            )),
            VolumeDown => self.jukebox_operation(Intent::Gain(
                (self.config.default_volume as f64 - 5.0) / 100.0,
            )),
            SeekForward => self.jukebox_operation(Intent::Seek(
                (self.playback.elapsed + Duration::from_secs(10))
                    .min(self.playback.total.unwrap_or(Duration::MAX)),
            )),
            SeekBackward => self.jukebox_operation(Intent::Seek(
                self.playback
                    .elapsed
                    .saturating_sub(Duration::from_secs(10)),
            )),
            SeekTo(position) => self.jukebox_operation(Intent::Seek(
                (*position).min(self.playback.total.unwrap_or(Duration::MAX)),
            )),
            ClearQueue => self.jukebox_operation(Intent::Clear),
            RemoveFromQueue => self.jukebox_operation(Intent::Remove(self.queue.cursor)),
            Shuffle => self.jukebox_operation(Intent::Shuffle),
            InstantMix => {
                if let Some(song) = self.instant_mix_seed() {
                    self.jukebox_operation(Intent::Mix(Box::new(song)));
                } else {
                    self.flash_status(
                        "Jukebox Mix: select a recent track, Browse track or queue track",
                    );
                }
            }
            Select if self.active_tab == Tab::NowPlaying => {
                self.jukebox_operation(Intent::Play(self.queue.cursor))
            }
            Select if self.active_tab == Tab::Home && self.config.home_discovery => {
                if self.discovery.section == 2 {
                    self.dispatch_jukebox(&InstantMix);
                } else {
                    self.jukebox_add_selection(true, true);
                }
            }
            Select
                if self.active_tab == Tab::Browser
                    && self.browser_focus == BrowserColumn::Tracks =>
            {
                self.jukebox_add_selection(false, false)
            }
            Select
                if self.active_tab == Tab::Browser
                    && self.browser_browse_mode != BrowseMode::Files =>
            {
                return false
            }
            HomeAlbumPlay | AddAllToQueueReplaceAlbum => self.jukebox_add_selection(true, true),
            HomeAlbumAddToQueue | AddToQueue => self.jukebox_add_selection(false, false),
            AddAllToQueue if self.browser_focus != BrowserColumn::Artists => {
                self.jukebox_add_selection(false, true)
            }
            Navigate(_) | Back | FocusLeft | FocusRight | SwitchTab | SwitchTabReverse
            | GoToHome | GoToBrowser | GoToNowPlaying | HomeSectionNext | HomeSectionPrev
            | HomeAlbumLeft | HomeAlbumRight | HomeRefresh | SearchStart | SearchInput(_)
            | SearchBackspace | SearchConfirm | SearchCancel | ToggleHelp | HelpScrollUp
            | HelpScrollDown | ToggleDynamicTheme | ToggleLyrics | ToggleFavorite | RateSong(_)
            | CheckConnection | Quit | None => return false,
            _ => self.flash_status("Not available in Jukebox yet; F8 returns to Local output"),
        }
        true
    }
}
