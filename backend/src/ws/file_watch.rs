use axum::extract::ws::{Message, WebSocket};
use futures::{sink::SinkExt, stream::StreamExt};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher, EventKind};
use notify::event::{ModifyKind, RenameMode};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, RwLock};

use crate::models::file::{FileChangeEvent, FileWatchMessage, SubscribeRequest};

// 标准化路径（统一使用正斜杠）
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

// 重命名事件状态跟踪
#[derive(Debug, Clone)]
pub struct RenameState {
    old_path: String,
    is_directory: bool,
}

pub struct FileWatcherManager {
    pub watch_tx: broadcast::Sender<FileChangeEvent>,
    pub watched_paths: Arc<RwLock<HashSet<String>>>,
    pub watchers: Arc<Mutex<Vec<RecommendedWatcher>>>,
    pub runtime_handle: tokio::runtime::Handle,
    pub pending_rename: Arc<Mutex<Option<RenameState>>>,
}

impl FileWatcherManager {
    pub fn new(watch_tx: broadcast::Sender<FileChangeEvent>) -> Self {
        Self {
            watch_tx,
            watched_paths: Arc::new(RwLock::new(HashSet::new())),
            watchers: Arc::new(Mutex::new(Vec::new())),
            runtime_handle: tokio::runtime::Handle::current(),
            pending_rename: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn add_watch(&self, path: String) -> notify::Result<()> {
        let mut paths = self.watched_paths.write().await;
        if paths.contains(&path) {
            return Ok(());
        }
        paths.insert(path.clone());
        drop(paths);

        let tx = self.watch_tx.clone();
        let runtime_handle = self.runtime_handle.clone();
        let path_for_log = path.clone();
        let pending_rename = self.pending_rename.clone();

        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                if let Ok(event) = res {
                    let tx = tx.clone();
                    let paths = event.paths.clone();
                    let kind = event.kind.clone();
                    let pending_rename = pending_rename.clone();

                    runtime_handle.spawn(async move {
                        match kind {
                            // 处理重命名事件
                            EventKind::Modify(ModifyKind::Name(rename_mode)) => {
                                match rename_mode {
                                    RenameMode::From => {
                                        // 缓存旧路径信息
                                        if let Some(path) = paths.first() {
                                            if let Some(path_str) = path.to_str() {
                                                let state = RenameState {
                                                    old_path: normalize_path(path_str),
                                                    is_directory: path.is_dir(),
                                                };
                                                let mut pending = pending_rename.lock().unwrap();
                                                *pending = Some(state);
                                            }
                                        }
                                    }
                                    RenameMode::To => {
                                        // 获取新路径，与缓存的旧路径配对发送重命名事件
                                        if let Some(path) = paths.first() {
                                            if let Some(path_str) = path.to_str() {
                                                let mut pending = pending_rename.lock().unwrap();
                                                if let Some(old_state) = pending.take() {
                                                    let new_name = path.file_name()
                                                        .and_then(|n| n.to_str())
                                                        .unwrap_or("")
                                                        .to_string();
                                                    let evt = FileChangeEvent::Renamed {
                                                        old_path: old_state.old_path,
                                                        new_path: normalize_path(path_str),
                                                        name: new_name,
                                                        is_directory: old_state.is_directory,
                                                    };
                                                    let _ = tx.send(evt);
                                                }
                                            }
                                        }
                                    }
                                    RenameMode::Both => {
                                        // 同时包含新旧路径
                                        if paths.len() >= 2 {
                                            if let (Some(old_path), Some(new_path)) = (paths.get(0), paths.get(1)) {
                                                if let (Some(old_str), Some(new_str)) = (old_path.to_str(), new_path.to_str()) {
                                                    let new_name = new_path.file_name()
                                                        .and_then(|n| n.to_str())
                                                        .unwrap_or("")
                                                        .to_string();
                                                    let evt = FileChangeEvent::Renamed {
                                                        old_path: normalize_path(old_str),
                                                        new_path: normalize_path(new_str),
                                                        name: new_name,
                                                        is_directory: new_path.is_dir(),
                                                    };
                                                    let _ = tx.send(evt);
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            // 处理其他事件类型
                            _ => {
                                for path in paths {
                                    if let Some(path_str) = path.to_str() {
                                        let normalized_path = normalize_path(path_str);
                                        let name = path
                                            .file_name()
                                            .and_then(|n| n.to_str())
                                            .unwrap_or("")
                                            .to_string();

                                        let change_event = match kind {
                                            EventKind::Create(_) => {
                                                Some(FileChangeEvent::Created {
                                                    path: normalized_path.clone(),
                                                    name: name.clone(),
                                                    is_directory: path.is_dir(),
                                                })
                                            }
                                            EventKind::Modify(ModifyKind::Data(_)) |
                                            EventKind::Modify(ModifyKind::Metadata(_)) => {
                                                Some(FileChangeEvent::Modified {
                                                    path: normalized_path.clone(),
                                                    name: name.clone(),
                                                })
                                            }
                                            EventKind::Remove(_) => {
                                                Some(FileChangeEvent::Deleted {
                                                    path: normalized_path.clone(),
                                                    name: name.clone(),
                                                })
                                            }
                                            _ => None,
                                        };

                                        if let Some(evt) = change_event {
                                            let _ = tx.send(evt);
                                        }
                                    }
                                }
                            }
                        }
                    });
                }
            },
            Config::default(),
        )?;

        watcher.watch(std::path::Path::new(&path), RecursiveMode::Recursive)?;
        println!("Added file watcher for path: {}", path_for_log);

        self.watchers.lock().unwrap().push(watcher);
        Ok(())
    }
}

pub async fn handle_file_watch_socket(
    socket: WebSocket,
    watch_tx: broadcast::Sender<FileChangeEvent>,
    watcher_manager: Arc<FileWatcherManager>,
) {
    println!("New file watch WebSocket connection established");

    let (mut sender, mut receiver) = socket.split();
    let mut watch_rx = watch_tx.subscribe();

    let send_task = tokio::spawn(async move {
        loop {
            match watch_rx.recv().await {
                Ok(event) => {
                    let msg = FileWatchMessage {
                        msg_type: "file_change".to_string(),
                        event,
                    };
                    if let Ok(json) = serde_json::to_string(&msg) {
                        if sender.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(_) => continue,
            }
        }
    });

    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    if let Ok(req) = serde_json::from_str::<SubscribeRequest>(&text) {
                        println!("Client subscribed to path: {}", req.path);
                        if let Err(e) = watcher_manager.add_watch(req.path).await {
                            eprintln!("Failed to add watcher: {}", e);
                        }
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }

    println!("File watch WebSocket connection closed");
}
