//! Boundary for commands sent to the local audio engine.

use ratune_player::PlayerCommand;
use std::sync::mpsc;

pub struct LocalOutput {
    tx: mpsc::Sender<PlayerCommand>,
}

impl LocalOutput {
    pub fn new(tx: mpsc::Sender<PlayerCommand>) -> Self {
        Self { tx }
    }

    pub fn send(&self, command: PlayerCommand) -> Result<(), mpsc::SendError<PlayerCommand>> {
        self.tx.send(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
