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

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3001").await.unwrap();
    println!("Server running on http://localhost:3001");
    println!("WebSocket endpoint: ws://localhost:3001/ws");
    println!("File watch WebSocket endpoint: ws://localhost:3001/ws/files");

    axum::serve(listener, app).await.unwrap();
}
