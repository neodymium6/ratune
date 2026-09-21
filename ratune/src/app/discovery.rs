use super::*;

impl App {
    pub(super) fn rebuild_discovery_local(&mut self) {
        let catalog = self
            .index_browse
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .albums_by_artist
                    .values()
                    .flatten()
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        self.discovery.rebuild_local(
            &self.history,
            &self.library_index_by_id,
            catalog,
            PlayRecord::now_secs(),
        );
    }

    pub(super) fn refresh_discovery(&mut self, force: bool) {
        self.rebuild_discovery_local();
        if !self.remote_available() {
            if self.discovery.newest.is_empty() {
                self.discovery.error = Some("Offline · reconnect and press r".into());
            }
            return;
        }
        if self.discovery.loading
            || (!force
                && self
                    .discovery
                    .refreshed
                    .is_some_and(|t| t.elapsed() < Duration::from_secs(300)))
        {
            return;
        }
        self.discovery.loading = true;
        self.discovery.request_id += 1;
        let request_id = self.discovery.request_id;
        let client = self.subsonic.clone();
        let tx = self.library_tx.clone();
        tokio::spawn(async move {
            let (newest, candidates) = tokio::join!(
                client.get_discovery_albums(true, 24),
                client.get_discovery_albums(false, 100)
            );
            let _ = tx
                .send(LibraryUpdate::DiscoveryShelves {
                    request_id,
                    newest: newest.map_err(|_| "Could not load new additions".into()),
                    candidates: candidates
                        .map_err(|_| "Could not load discovery candidates".into()),
                })
                .await;
        });
    }

    pub(super) fn cancel_discovery_album(&mut self) -> bool {
        let cancelled = self.discovery_album_request.cancel();
        if let Some(task) = self.discovery_album_task.take() {
            task.abort();
        }
        cancelled
    }

    pub(super) fn start_discovery_album(&mut self, replace: bool) {
        let Some(album) = self.discovery.selected_album() else {
            return;
        };
        let id = album.id.clone();
        self.cancel_discovery_album();
        self.instant_mix.cancel();
        if let Some(task) = self.instant_mix_task.take() {
            task.abort();
        }
        let cached = self
            .index_browse
            .as_ref()
            .and_then(|s| s.tracks_by_album.get(&id))
            .cloned()
            .or_else(|| match self.library.tracks.get(&id) {
                Some(LoadingState::Loaded(s)) => Some(s.clone()),
                _ => None,
            });
        if !self.remote_available() && cached.as_ref().is_none_or(|songs| songs.is_empty()) {
            self.flash_status("Offline — album tracks unavailable");
            return;
        }
        let request_id = self.discovery_album_request.start(
            replace,
            &self.queue,
            self.play_gen,
            self.playback.paused,
        );
        if let Some(songs) = cached.filter(|s| !s.is_empty()) {
            self.apply_library_update(LibraryUpdate::DiscoveryAlbum {
                request_id,
                result: Ok(songs),
            });
            return;
        }
        let client = self.subsonic.clone();
        let tx = self.library_tx.clone();
        self.flash_status_secs(
            "Loading selected album… Esc cancels; queue unchanged until ready",
            30,
        );
        self.discovery_album_task = Some(
            tokio::spawn(async move {
                let result = client
                    .get_album(&id)
                    .await
                    .map(|a| a.song)
                    .map_err(|_| "Cannot load album — queue unchanged".to_string());
                let _ = tx
                    .send(LibraryUpdate::DiscoveryAlbum { request_id, result })
                    .await;
            })
            .abort_handle(),
        );
    }

    pub(super) fn dispatch_discovery(&mut self, action: &Action) -> bool {
        match action {
            Action::HomeSectionNext => self.discovery.section = (self.discovery.section + 1) % 3,
            Action::HomeSectionPrev => self.discovery.section = (self.discovery.section + 2) % 3,
            Action::Navigate(dir) => self.discovery.move_selection(*dir),
            Action::HomeAlbumLeft | Action::FocusLeft => {
                self.discovery.move_selection(Direction::Up)
            }
            Action::HomeAlbumRight | Action::FocusRight => {
                self.discovery.move_selection(Direction::Down)
            }
            Action::HomeRefresh => self.refresh_discovery(true),
            Action::Select | Action::HomeAlbumPlay => {
                if self.discovery.section == 2 {
                    self.handle_instant_mix();
                } else {
                    self.start_discovery_album(true);
                }
            }
            Action::HomeAlbumAddToQueue | Action::AddToQueue | Action::AddAllToQueue => {
                if let Some(song) = self.discovery.selected_seed().cloned() {
                    self.cancel_discovery_album();
                    let start = self.queue.songs.is_empty();
                    self.queue.push(song);
                    if start {
                        self.queue.cursor = 0;
                        self.play_current();
                    }
                    self.flash_status("Added selected track to queue");
                } else {
                    self.start_discovery_album(false);
                }
            }
            Action::Back => {
                if self.cancel_discovery_album() {
                    self.flash_status("Album action cancelled — queue unchanged");
                }
            }
            Action::SearchStart => self.flash_status("Use 2 to browse and filter the library"),
            _ => return false,
        }
        true
    }

    pub(super) fn apply_discovery_update(&mut self, update: LibraryUpdate) {
        match update {
            LibraryUpdate::DiscoveryShelves {
                request_id,
                newest,
                candidates,
            } => {
                if request_id != self.discovery.request_id {
                    return;
                }
                self.discovery.loading = false;
                self.discovery.refreshed = Some(Instant::now());
                match newest {
                    Ok(albums) => {
                        self.discovery.set_newest(albums);
                        self.discovery.error = None;
                    }
                    Err(_) => {
                        self.discovery.error = Some("Cannot load new additions · r to retry".into())
                    }
                }
                if let Ok(albums) = candidates {
                    self.discovery.candidates = albums;
                }
                self.rebuild_discovery_local();
            }
            LibraryUpdate::DiscoveryAlbum { request_id, result } => {
                match self.discovery_album_request.finish(
                    request_id,
                    &self.queue,
                    self.play_gen,
                    self.playback.paused,
                    result,
                ) {
                    Ok(Some((replace, songs))) => {
                        self.discovery_album_task = None;
                        let count = songs.len();
                        let start = replace || self.queue.songs.is_empty();
                        if replace {
                            self.handle_clear_queue();
                        }
                        for song in songs {
                            self.queue.push(song);
                        }
                        if start {
                            self.queue.cursor = 0;
                            self.queue.scroll = 0;
                            self.active_tab = Tab::NowPlaying;
                            self.play_current();
                        }
                        self.flash_status_secs(
                            format!(
                                "Album: {} {count} tracks",
                                if start { "playing" } else { "queued" }
                            ),
                            5,
                        );
                    }
                    Ok(None) => {}
                    Err(message) => {
                        self.discovery_album_task = None;
                        self.flash_status_secs(message, 5);
                    }
                }
            }
            _ => unreachable!("expected discovery update"),
        }
    }
}
