//! One writer and a four-item queue; failed work retains the recording in memory.
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock, mpsc},
};
#[derive(Debug)]
pub struct Failure {
    pub message: String,
    pub run: Arc<pelagic::experiment::Recording>,
}
pub type Reply = Result<PathBuf, Failure>;
type Task = (
    PathBuf,
    Arc<pelagic::experiment::Recording>,
    mpsc::Sender<Reply>,
);
pub fn submit(path: PathBuf, run: Arc<pelagic::experiment::Recording>) -> mpsc::Receiver<Reply> {
    static WRITER: OnceLock<mpsc::SyncSender<Task>> = OnceLock::new();
    let writer = WRITER.get_or_init(|| {
        let (send, receive) = mpsc::sync_channel::<Task>(4);
        std::thread::spawn(move || {
            while let Ok((path, run, reply)) = receive.recv() {
                let result = pelagic::experiment::write_atomic(&path, &run)
                    .map(|_| path)
                    .map_err(|message| Failure { message, run });
                let _ = reply.send(result);
            }
        });
        send
    });
    let (send, receive) = mpsc::channel();
    if let Err(mpsc::SendError((_, run, reply))) = writer.send((path, run, send)) {
        let _ = reply.send(Err(Failure {
            message: "Backup writer stopped".into(),
            run,
        }));
    }
    receive
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_backup_retains_the_run_and_can_be_retried() {
        let root = std::env::temp_dir().join(format!(
            "pelagic-backup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let blocker = root.join("not-a-directory");
        std::fs::write(&blocker, b"block").unwrap();
        let run = Arc::new(pelagic::experiment::Recording::new(
            pelagic::scene::Scene::default().value().to_string(),
            1.0 / 60.0,
        ));
        let error = submit(blocker.join("run.pelagic-run"), run.clone())
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap()
            .unwrap_err();
        assert!(Arc::ptr_eq(&run, &error.run));
        let path = submit(root.join("saved.pelagic-run"), error.run)
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap()
            .unwrap();
        let loaded =
            pelagic::experiment::Recording::read(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(*run, loaded);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(blocker).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
