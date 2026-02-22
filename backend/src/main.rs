mod handlers;
mod models;
mod rdp;
mod routes;
mod ws;

use std::sync::Arc;
use tokio::sync::broadcast;

use crate::models::file::FileChangeEvent;
use crate::routes::create_router;
use crate::ws::file_watch::FileWatcherManager;

#[tokio::main]
async fn main() {
    let current_dir = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    println!("Current working directory: {}", current_dir);

    let (watch_tx, _) = broadcast::channel::<FileChangeEvent>(100);

    let watcher_manager = Arc::new(FileWatcherManager::new(watch_tx.clone()));

    if let Err(e) = watcher_manager.add_watch(current_dir.clone()).await {
        eprintln!("Failed to add default watcher: {}", e);
    }

    let app = create_router(watcher_manager, watch_tx);

    let listener = match tokio::net::TcpListener::bind("127.0.0.1:3003").await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind to port 3003: {}", e);
            std::process::exit(1);
        }
    };
    println!("Server running on http://localhost:3003");
    println!("WebSocket endpoint: ws://localhost:3003/ws");
    println!("File watch WebSocket endpoint: ws://localhost:3003/ws/files");

    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("Server error: {}", e);
        std::process::exit(1);
    }
}
