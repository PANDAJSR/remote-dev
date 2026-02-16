use axum::{
    response::Json,
    routing::get,
    Router,
};
use serde_json::{json, Value};
use tower_http::services::ServeDir;

async fn health_check() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "message": "Server is running"
    }))
}

async fn api_hello() -> Json<Value> {
    Json(json!({
        "message": "Hello from API"
    }))
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/api/health", get(health_check))
        .route("/api/hello", get(api_hello))
        .fallback_service(ServeDir::new("static"));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    
    println!("Server running on http://0.0.0.0:3000");
    println!("API endpoints:");
    println!("  - GET /api/health");
    println!("  - GET /api/hello");
    println!("Static files served from /static directory");
    
    axum::serve(listener, app).await.unwrap();
}