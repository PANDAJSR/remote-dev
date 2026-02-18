use axum::{
    extract::WebSocketUpgrade,
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tokio::sync::broadcast;
use tower_http::services::ServeDir;

use crate::handlers;
use crate::models::file::FileChangeEvent;
use crate::ws;
use crate::ws::file_watch::FileWatcherManager;
use crate::rdp::signaling::{SignalingState, create_session, handle_offer, handle_ice_candidate, get_ice_candidates, get_server_info};

pub fn create_router(
    watcher_manager: Arc<FileWatcherManager>,
    watch_tx: broadcast::Sender<FileChangeEvent>,
) -> Router {
    let watcher_manager_for_files = watcher_manager.clone();

    // 创建 RDP 信令状态
    let rdp_state = Arc::new(SignalingState::new());
    let rdp_state_for_routes = Arc::clone(&rdp_state);

    Router::new()
        // WebSocket 终端
        .route("/ws", get(ws::terminal::ws_handler))
        .route(
            "/ws/files",
            get(move |ws: WebSocketUpgrade| {
                let tx = watch_tx.clone();
                let manager = watcher_manager_for_files.clone();
                async move {
                    ws.on_upgrade(move |socket| ws::file_watch::handle_file_watch_socket(socket, tx, manager))
                }
            }),
        )
        // RDP (远程桌面) API
        .route("/api/rdp/info", get(get_server_info))
        .route(
            "/api/rdp/session",
            post(create_session).with_state(Arc::clone(&rdp_state_for_routes)),
        )
        .route(
            "/api/rdp/offer",
            post(handle_offer).with_state(Arc::clone(&rdp_state_for_routes)),
        )
        .route(
            "/api/rdp/ice",
            post(handle_ice_candidate).with_state(Arc::clone(&rdp_state_for_routes)),
        )
        .route(
            "/api/rdp/ice-candidates",
            post(get_ice_candidates).with_state(Arc::clone(&rdp_state_for_routes)),
        )
        // 原有 API
        .route("/api/health", get(handlers::health::health))
        .route("/api/tree", get(handlers::file::get_directory_tree))
        .route("/api/children", get(handlers::file::get_children))
        .route(
            "/api/file",
            get(handlers::file::get_file_content)
                .post(handlers::file::save_file_content)
                .delete(handlers::file::delete_file),
        )
        .route("/api/file/rename", axum::routing::put(handlers::file::rename_file))
        .route("/api/file/copy", axum::routing::post(handlers::file::copy_file))
        .route("/api/folder", axum::routing::post(handlers::file::create_folder))
        .fallback_service(ServeDir::new("static"))
}
