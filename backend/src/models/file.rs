use serde::{Deserialize, Serialize};

// 文件变更事件类型
#[derive(Debug, Serialize, Clone)]
#[serde(tag = "event")]
pub enum FileChangeEvent {
    #[serde(rename = "created")]
    Created {
        path: String,
        name: String,
        is_directory: bool,
    },
    #[serde(rename = "modified")]
    Modified { path: String, name: String },
    #[serde(rename = "deleted")]
    Deleted { path: String, name: String },
    #[serde(rename = "renamed")]
    Renamed {
        old_path: String,
        new_path: String,
        name: String,
        is_directory: bool,
    },
}

// 文件监控消息
#[derive(Debug, Serialize)]
pub struct FileWatchMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(flatten)]
    pub event: FileChangeEvent,
}

#[derive(Debug, Serialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub children: Option<Vec<FileEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct DirQuery {
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FileQuery {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct SaveFileRequest {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameFileRequest {
    pub old_path: String,
    pub new_path: String,
}

#[derive(Debug, Deserialize)]
pub struct CopyFileRequest {
    pub source_path: String,
    pub target_path: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteFileRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateFolderRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct FileContent {
    pub path: String,
    pub content: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct SaveResponse {
    pub success: bool,
    pub message: String,
}

// 客户端订阅的路径
#[derive(Debug, Deserialize)]
pub struct SubscribeRequest {
    pub path: String,
}
