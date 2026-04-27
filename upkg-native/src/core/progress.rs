#[derive(Debug, Clone)]
pub enum InstallProgress {
    DownloadStarted {
        name: String,
        total_bytes: Option<u64>,
    },
    DownloadProgress {
        name: String,
        downloaded: u64,
        total_bytes: Option<u64>,
    },
    DownloadCompleted {
        name: String,
        total_bytes: u64,
    },
    UnpackStarted {
        name: String,
    },
    UnpackCompleted {
        name: String,
    },
    LinkStarted {
        name: String,
    },
    LinkCompleted {
        name: String,
    },
    LinkSkipped {
        name: String,
        reason: String,
    },
    InstallCompleted {
        name: String,
    },
}

pub type ProgressCallback = Box<dyn Fn(InstallProgress) + Send + Sync>;
