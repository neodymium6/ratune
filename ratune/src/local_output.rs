//! Boundary for commands sent to the local audio engine.

use ratune_player::PlayerCommand;
use std::cell::Cell;
use std::sync::mpsc;

pub struct LocalOutput {
    tx: mpsc::Sender<PlayerCommand>,
    remote: Cell<bool>,
}

impl LocalOutput {
    pub fn new(tx: mpsc::Sender<PlayerCommand>) -> Self {
        Self {
            tx,
            remote: Cell::new(false),
        }
    }

    /// Stop local playback before blocking all subsequent commands except shutdown.
    pub fn set_remote(&self, remote: bool) {
        if remote {
            let _ = self.tx.send(PlayerCommand::Stop);
        }
        self.remote.set(remote);
    }

    pub fn send(&self, command: PlayerCommand) -> Result<(), mpsc::SendError<PlayerCommand>> {
        if self.remote.get() && !matches!(command, PlayerCommand::Quit) {
            return Ok(());
        }
        self.tx.send(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_mode_blocks_local_playback_until_explicit_disconnect() {
        let (tx, rx) = mpsc::channel();
        let output = LocalOutput::new(tx);
        output.set_remote(true);
        assert!(matches!(rx.recv().unwrap(), PlayerCommand::Stop));
        output.send(PlayerCommand::Resume).unwrap();
        output.send(PlayerCommand::SetVolume(1.0)).unwrap();
        output
            .send(PlayerCommand::Seek(std::time::Duration::from_secs(10)))
            .unwrap();
        output
            .send(PlayerCommand::PlayUrl {
                url: "http://127.0.0.1/fixture".into(),
                duration: None,
                gen: 1,
            })
            .unwrap();
        assert!(rx.try_recv().is_err());
        output.set_remote(false);
        output.send(PlayerCommand::Resume).unwrap();
        assert!(matches!(rx.recv().unwrap(), PlayerCommand::Resume));
    }

    #[test]
    fn local_engine_can_shut_down_while_remote_playback_continues() {
        let (tx, rx) = mpsc::channel();
        let output = LocalOutput::new(tx);
        output.set_remote(true);
        assert!(matches!(rx.recv().unwrap(), PlayerCommand::Stop));
        output.send(PlayerCommand::Quit).unwrap();
        assert!(matches!(rx.recv().unwrap(), PlayerCommand::Quit));
    }

    #[test]
    fn forwards_commands_in_order() {
        let (tx, rx) = mpsc::channel();
        let output = LocalOutput::new(tx);
        output.send(PlayerCommand::SetVolume(0.25)).unwrap();
        output.send(PlayerCommand::Resume).unwrap();
        output.send(PlayerCommand::Stop).unwrap();
        output.send(PlayerCommand::Quit).unwrap();
        assert!(matches!(rx.recv().unwrap(), PlayerCommand::SetVolume(v) if v == 0.25));
        assert!(matches!(rx.recv().unwrap(), PlayerCommand::Resume));
        assert!(matches!(rx.recv().unwrap(), PlayerCommand::Stop));
        assert!(matches!(rx.recv().unwrap(), PlayerCommand::Quit));
    }

    #[test]
    fn preserves_channel_errors() {
        let (tx, rx) = mpsc::channel();
        drop(rx);
        let error = LocalOutput::new(tx).send(PlayerCommand::Stop).unwrap_err();
        assert!(matches!(error.0, PlayerCommand::Stop));
    }
}
