use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, Query},
    response::IntoResponse,
    routing::get,
    Router,
    Json,
};
use futures::{sink::SinkExt, stream::StreamExt};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tower_http::services::ServeDir;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum WsMessage {
    #[serde(rename = "input")]
    Input { data: String },
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },
    #[serde(rename = "output")]
    Output { data: String },
}

#[derive(Debug, Serialize)]
struct FileEntry {
    name: String,
    path: String,
    is_directory: bool,
    children: Option<Vec<FileEntry>>,
}

#[derive(Debug, Deserialize)]
struct DirQuery {
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileQuery {
    path: String,
}

#[derive(Debug, Deserialize)]
struct SaveFileRequest {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct RenameFileRequest {
    old_path: String,
    new_path: String,
}

#[derive(Debug, Deserialize)]
struct CopyFileRequest {
    source_path: String,
    target_path: String,
}

#[derive(Debug, Deserialize)]
struct DeleteFileRequest {
    path: String,
}

#[derive(Debug, Deserialize)]
struct CreateFolderRequest {
    path: String,
}

#[derive(Debug, Serialize)]
struct FileContent {
    path: String,
    content: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct SaveResponse {
    success: bool,
    message: String,
}

async fn get_directory_tree(Query(query): Query<DirQuery>) -> impl IntoResponse {
    let path = match query.path {
        Some(p) => p,
        None => {
            // 使用当前工作目录作为默认根目录
            match std::env::current_dir() {
                Ok(dir) => dir.to_string_lossy().to_string(),
                Err(_) => {
                    return Json(serde_json::json!({
                        "error": "Failed to get current directory"
                    })).into_response();
                }
            }
        }
    };

    let tree = build_tree(&path, 0);
    Json(tree).into_response()
}

// 获取指定目录的直接子项（用于懒加载）
async fn get_children(Query(query): Query<DirQuery>) -> impl IntoResponse {
    let path = query.path.unwrap_or_default();

    if path.is_empty() {
        return Json(serde_json::json!({
            "error": "Path is required"
        })).into_response();
    }

    let path_obj = std::path::Path::new(&path);

    if !path_obj.is_dir() {
        return Json(serde_json::json!({
            "error": "Path is not a directory"
        })).into_response();
    }

    match std::fs::read_dir(&path) {
        Ok(entries) => {
            let mut children: Vec<FileEntry> = entries
                .filter_map(|entry| {
                    entry.ok().map(|e| {
                        let path = e.path();
                        let path_str = path.to_string_lossy().to_string();
                        let name = e.file_name().to_string_lossy().to_string();
                        let is_directory = path.is_dir();

                        // 对于子目录，不预加载其子项（设为 None）
                        FileEntry {
                            name,
                            path: path_str,
                            is_directory,
                            children: if is_directory { Some(vec![]) } else { None },
                        }
                    })
                })
                .collect();

            children.sort_by(|a, b| {
                match (a.is_directory, b.is_directory) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.name.cmp(&b.name),
                }
            });

            Json(children).into_response()
        }
        Err(e) => {
            Json(serde_json::json!({
                "error": format!("Failed to read directory: {}", e)
            })).into_response()
        }
    }
}

fn build_tree(path: &str, depth: u8) -> FileEntry {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string();
    
    let is_directory = std::path::Path::new(path).is_dir();
    
    let children = if is_directory && depth < 3 {
        match std::fs::read_dir(path) {
            Ok(entries) => {
                let mut children_vec: Vec<FileEntry> = entries
                    .filter_map(|entry| {
                        entry.ok().map(|e| {
                            let path = e.path();
                            let path_str = path.to_string_lossy().to_string();
                            build_tree(&path_str, depth + 1)
                        })
                    })
                    .collect();
                children_vec.sort_by(|a, b| {
                    match (a.is_directory, b.is_directory) {
                        (true, false) => std::cmp::Ordering::Less,
                        (false, true) => std::cmp::Ordering::Greater,
                        _ => a.name.cmp(&b.name),
                    }
                });
                Some(children_vec)
            }
            Err(_) => None,
        }
    } else {
        None
    };
    
    FileEntry {
        name,
        path: path.to_string(),
        is_directory,
        children,
    }
}

async fn get_file_content(Query(query): Query<FileQuery>) -> impl IntoResponse {
    let path = query.path;
    
    // 安全检查：确保路径是有效的
    let path_obj = std::path::Path::new(&path);
    
    // 检查是否是文件
    if !path_obj.is_file() {
        return Json(serde_json::json!({
            "error": "Path is not a file or does not exist"
        })).into_response();
    }
    
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let name = path_obj
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            
            let file_content = FileContent {
                path: path.clone(),
                content,
                name,
            };
            Json(file_content).into_response()
        }
        Err(e) => {
            Json(serde_json::json!({
                "error": format!("Failed to read file: {}", e)
            })).into_response()
        }
    }
}

async fn save_file_content(Json(payload): Json<SaveFileRequest>) -> impl IntoResponse {
    let path = payload.path;

    // 安全检查：确保路径是有效的
    let path_obj = std::path::Path::new(&path);

    // 确保父目录存在
    if let Some(parent) = path_obj.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Json(SaveResponse {
                success: false,
                message: format!("Failed to create directory: {}", e),
            }).into_response();
        }
    }

    match std::fs::write(&path, payload.content) {
        Ok(_) => Json(SaveResponse {
            success: true,
            message: "File saved successfully".to_string(),
        }).into_response(),
        Err(e) => Json(SaveResponse {
            success: false,
            message: format!("Failed to write file: {}", e),
        }).into_response(),
    }
}

async fn rename_file(Json(payload): Json<RenameFileRequest>) -> impl IntoResponse {
    // 安全检查：确保源路径存在
    let old_path_obj = std::path::Path::new(&payload.old_path);
    if !old_path_obj.exists() {
        return Json(SaveResponse {
            success: false,
            message: "Source path does not exist".to_string(),
        }).into_response();
    }

    // 确保目标父目录存在
    let new_path_obj = std::path::Path::new(&payload.new_path);
    if let Some(parent) = new_path_obj.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Json(SaveResponse {
                success: false,
                message: format!("Failed to create directory: {}", e),
            }).into_response();
        }
    }

    match std::fs::rename(&payload.old_path, &payload.new_path) {
        Ok(_) => Json(SaveResponse {
            success: true,
            message: "File renamed successfully".to_string(),
        }).into_response(),
        Err(e) => Json(SaveResponse {
            success: false,
            message: format!("Failed to rename file: {}", e),
        }).into_response(),
    }
}

async fn delete_file(Query(query): Query<DeleteFileRequest>) -> impl IntoResponse {
    let path = query.path;
    let path_obj = std::path::Path::new(&path);

    if !path_obj.exists() {
        return Json(SaveResponse {
            success: false,
            message: "Path does not exist".to_string(),
        }).into_response();
    }

    let result = if path_obj.is_dir() {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_file(&path)
    };

    match result {
        Ok(_) => Json(SaveResponse {
            success: true,
            message: "File deleted successfully".to_string(),
        }).into_response(),
        Err(e) => Json(SaveResponse {
            success: false,
            message: format!("Failed to delete file: {}", e),
        }).into_response(),
    }
}

async fn create_folder(Json(payload): Json<CreateFolderRequest>) -> impl IntoResponse {
    let path = payload.path;
    let path_obj = std::path::Path::new(&path);

    // 如果路径已存在，返回错误
    if path_obj.exists() {
        return Json(SaveResponse {
            success: false,
            message: "Path already exists".to_string(),
        }).into_response();
    }

    // 创建目录
    match std::fs::create_dir_all(&path) {
        Ok(_) => Json(SaveResponse {
            success: true,
            message: "Folder created successfully".to_string(),
        }).into_response(),
        Err(e) => Json(SaveResponse {
            success: false,
            message: format!("Failed to create folder: {}", e),
        }).into_response(),
    }
}

async fn copy_file(Json(payload): Json<CopyFileRequest>) -> impl IntoResponse {
    // 安全检查：确保源路径存在
    let source_path_obj = std::path::Path::new(&payload.source_path);
    if !source_path_obj.exists() {
        return Json(SaveResponse {
            success: false,
            message: "Source path does not exist".to_string(),
        }).into_response();
    }

    // 确保目标父目录存在
    let target_path_obj = std::path::Path::new(&payload.target_path);
    if let Some(parent) = target_path_obj.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Json(SaveResponse {
                success: false,
                message: format!("Failed to create directory: {}", e),
            }).into_response();
        }
    }

    let result = if source_path_obj.is_dir() {
        // 递归复制目录
        copy_dir_recursive(&payload.source_path, &payload.target_path)
    } else {
        std::fs::copy(&payload.source_path, &payload.target_path).map(|_| ())
    };

    match result {
        Ok(_) => Json(SaveResponse {
            success: true,
            message: "File copied successfully".to_string(),
        }).into_response(),
        Err(e) => Json(SaveResponse {
            success: false,
            message: format!("Failed to copy file: {}", e),
        }).into_response(),
    }
}

fn copy_dir_recursive(src: &str, dst: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = std::path::Path::new(dst).join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(src_path.to_str().unwrap(), dst_path.to_str().unwrap())?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

async fn handle_socket(socket: WebSocket) {
    println!("New WebSocket connection established");

    // 创建 PTY
    let pty_system = NativePtySystem::default();

    let pair = pty_system
        .openpty(PtySize {
            rows: 30,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("Failed to open PTY");

    // 启动 shell
    let shell = if cfg!(windows) {
        // Windows: 尝试找 Git Bash，找不到再用 PowerShell
        let git_bash_paths = [
            r"C:/Program Files/Git/bin/bash.exe",
            r"C:/Program Files/Git/usr/bin/bash.exe",
            r"C:/Program Files (x86)/Git/bin/bash.exe",
        ];
        let found_shell = git_bash_paths
            .iter()
            .find(|&&path| std::path::Path::new(path).exists())
            .map(|&s| s.to_string());
        
        if let Some(ref s) = found_shell {
            println!("Found Git Bash at: {}", s);
        } else {
            println!("Git Bash not found, using PowerShell");
        }
        
        found_shell.unwrap_or_else(|| "powershell.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string())
    };

    let mut cmd = CommandBuilder::new(&shell);
    // 尝试使用 screen-256color 可能更兼容
    cmd.env("TERM", "screen-256color");
    cmd.env("TERM_PROGRAM", "screen");
    cmd.env("TERM_PROGRAM_VERSION", "0.1.0");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("SHELL", &shell);
    
    // 添加其他常用环境变量
    if let Ok(home) = std::env::var("HOME") {
        cmd.env("HOME", home);
    }
    if let Ok(user) = std::env::var("USER") {
        cmd.env("USER", user);
    }
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    if let Ok(lang) = std::env::var("LANG") {
        cmd.env("LANG", lang);
    }
    
    // 强制交互模式（对 bash/zsh 有效）
    if shell.contains("bash") {
        cmd.arg("-i");  // 交互模式
        cmd.arg("-l");  // 登录 shell（加载 .bash_profile）
        println!("Using bash with -i -l flags");
    }

    let _child = pair
        .slave
        .spawn_command(cmd)
        .expect("Failed to spawn shell");

    drop(pair.slave);

    // 使用 Arc<Mutex> 共享 master，以便 resize 可以访问
    let master = Arc::new(Mutex::new(pair.master));

    // 获取 reader
    let master_for_reader = Arc::clone(&master);
    let mut reader = {
        let master = master_for_reader.lock().unwrap();
        master.try_clone_reader().expect("Failed to clone reader")
    };

    // 获取 writer
    let master_for_writer = Arc::clone(&master);
    let mut writer = {
        let master = master_for_writer.lock().unwrap();
        master.take_writer().expect("Failed to take writer")
    };

    let (mut sender, mut receiver) = socket.split();

    // 用于从 PTY 读取发送数据到 WebSocket
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    // PTY 读取任务（在独立线程中）
    let tx_clone = tx.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(n) if n > 0 => {
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    if tx_clone.send(data).is_err() {
                        break;
                    }
                }
                Ok(_) => break,
                Err(e) => {
                    eprintln!("PTY read error: {}", e);
                    break;
                }
            }
        }
    });

    // 在独立线程中处理写入和 resize
    let (write_tx, mut write_rx) = mpsc::unbounded_channel::<String>();
    let (resize_tx, mut resize_rx) = mpsc::unbounded_channel::<(u16, u16)>();

    std::thread::spawn(move || {
        loop {
            // 检查是否有写入数据
            if let Ok(data) = write_rx.try_recv() {
                if writer.write_all(data.as_bytes()).is_err() {
                    break;
                }
                if writer.flush().is_err() {
                    break;
                }
            }

            // 检查是否有 resize 请求
            if let Ok((cols, rows)) = resize_rx.try_recv() {
                let master = master.lock().unwrap();
                if let Err(e) = master.resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                }) {
                    eprintln!("Failed to resize PTY: {}", e);
                } else {
                    println!("PTY resized to {}x{}", cols, rows);
                }
            }

            // 短暂休眠避免 CPU 占用过高
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    // WebSocket 发送任务
    let send_task = tokio::spawn(async move {
        while let Some(data) = rx.recv().await {
            let msg = WsMessage::Output { data };
            if let Ok(json) = serde_json::to_string(&msg) {
                if sender.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    });

    // WebSocket 接收任务
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    if let Ok(ws_msg) = serde_json::from_str::<WsMessage>(&text) {
                        match ws_msg {
                            WsMessage::Input { data } => {
                                let _ = write_tx.send(data);
                            }
                            WsMessage::Resize { cols, rows } => {
                                let _ = resize_tx.send((cols, rows));
                            }
                            _ => {}
                        }
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    // 等待任务结束
    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    println!("WebSocket connection closed");
}

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn health() -> impl IntoResponse {
    axum::Json(serde_json::json!({"status": "ok"}))
}

#[tokio::main]
async fn main() {
    // 获取并打印当前工作目录
    let current_dir = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    println!("Current working directory: {}", current_dir);
    
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/api/health", get(health))
        .route("/api/tree", get(get_directory_tree))
        .route("/api/children", get(get_children))
        .route("/api/file", get(get_file_content).post(save_file_content).delete(delete_file))
        .route("/api/file/rename", axum::routing::put(rename_file))
        .route("/api/file/copy", axum::routing::post(copy_file))
        .route("/api/folder", axum::routing::post(create_folder))
        .fallback_service(ServeDir::new("static"));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("Server running on http://localhost:3000");
    println!("WebSocket endpoint: ws://localhost:3000/ws");
    axum::serve(listener, app).await.unwrap();
}
