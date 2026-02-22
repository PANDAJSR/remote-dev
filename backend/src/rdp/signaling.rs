use axum::{
    extract::{Json, State},
    response::Json as JsonResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::rdp::webrtc::{WebrtcManager, SdpOffer, SdpAnswer, IceCandidate};

/// 信令服务器状态
#[derive(Clone)]
pub struct SignalingState {
    pub webrtc_manager: Arc<WebrtcManager>,
}

impl SignalingState {
    pub fn new() -> Self {
        Self {
            webrtc_manager: Arc::new(WebrtcManager::new()),
        }
    }
}

/// 创建新的 WebRTC 会话
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub resolution: Option<(u32, u32)>,
    pub fps: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateSessionResponse {
    pub session_id: String,
    pub message: String,
}

pub async fn create_session(
    State(state): State<Arc<SignalingState>>,
    Json(_req): Json<CreateSessionRequest>,
) -> JsonResponse<CreateSessionResponse> {
    let session_id = Uuid::new_v4().to_string();
    
    // 创建新的 WebRTC 会话
    match state.webrtc_manager.create_session(session_id.clone()).await {
        Ok(_) => {
            println!("Created new WebRTC session: {}", session_id);
            JsonResponse(CreateSessionResponse {
                session_id,
                message: "Session created successfully".to_string(),
            })
        }
        Err(e) => {
            eprintln!("Failed to create session: {}", e);
            JsonResponse(CreateSessionResponse {
                session_id: String::new(),
                message: format!("Failed: {}", e),
            })
        }
    }
}

/// 处理 SDP Offer 并返回 Answer
pub async fn handle_offer(
    State(state): State<Arc<SignalingState>>,
    Json(offer): Json<SdpOffer>,
) -> JsonResponse<SdpAnswer> {
    let session_id = offer.session_id.clone();
    
    // 获取会话
    let session = match state.webrtc_manager.get_session(&session_id).await {
        Some(s) => s,
        None => {
            eprintln!("Session not found: {}", session_id);
            return JsonResponse(SdpAnswer {
                sdp: String::new(),
                session_id: session_id.clone(),
                success: false,
            });
        }
    };

    // 处理 offer，创建 answer
    match session.handle_offer(offer.sdp).await {
        Ok(answer_sdp) => {
            println!("Created answer for session: {}", session_id);
            JsonResponse(SdpAnswer {
                sdp: answer_sdp,
                session_id,
                success: true,
            })
        }
        Err(e) => {
            eprintln!("Failed to create answer: {}", e);
            JsonResponse(SdpAnswer {
                sdp: String::new(),
                session_id,
                success: false,
            })
        }
    }
}

/// 处理 ICE Candidate
#[derive(Debug, Serialize, Deserialize)]
pub struct IceCandidateResponse {
    pub success: bool,
}

pub async fn handle_ice_candidate(
    State(state): State<Arc<SignalingState>>,
    Json(candidate_msg): Json<IceCandidate>,
) -> JsonResponse<IceCandidateResponse> {
    let session_id = candidate_msg.session_id.clone();
    
    let session = match state.webrtc_manager.get_session(&session_id).await {
        Some(s) => s,
        None => {
            return JsonResponse(IceCandidateResponse { success: false });
        }
    };

    // 添加 ICE candidate
    if let Err(e) = session.add_ice_candidate(candidate_msg).await {
        eprintln!("Failed to add ICE candidate: {}", e);
        return JsonResponse(IceCandidateResponse { success: false });
    }

    JsonResponse(IceCandidateResponse { success: true })
}

/// 获取服务器 ICE Candidates
#[derive(Debug, Serialize, Deserialize)]
pub struct GetIceCandidatesRequest {
    pub session_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetIceCandidatesResponse {
    pub candidates: Vec<IceCandidate>,
    pub success: bool,
}

pub async fn get_ice_candidates(
    State(state): State<Arc<SignalingState>>,
    Json(req): Json<GetIceCandidatesRequest>,
) -> JsonResponse<GetIceCandidatesResponse> {
    let session = match state.webrtc_manager.get_session(&req.session_id).await {
        Some(s) => s,
        None => {
            return JsonResponse(GetIceCandidatesResponse {
                candidates: vec![],
                success: false,
            });
        }
    };

    let candidates = session.get_ice_candidates().await;

    JsonResponse(GetIceCandidatesResponse {
        candidates,
        success: true,
    })
}

/// 获取服务器信息
#[derive(Debug, Serialize, Deserialize)]
pub struct ServerInfo {
    pub supports_webrtc: bool,
    pub screen_width: u32,
    pub screen_height: u32,
    pub status: String,
    pub message: String,
}

/// 服务器状态
#[derive(Debug, Serialize, Deserialize)]
pub struct ServerStatus {
    pub healthy: bool,
    pub display_available: bool,
    pub message: String,
}

pub async fn get_server_info() -> JsonResponse<ServerInfo> {
    println!("[RDP] Received server info request");
    
    // 使用健壮的方式获取服务器信息
    let server_status = check_server_health().await;
    
    // 获取屏幕分辨率（带多层错误处理）
    let (width, height) = match get_display_resolution_with_fallback().await {
        Ok((w, h)) => {
            println!("[RDP] Display resolution obtained: {}x{}", w, h);
            (w, h)
        }
        Err(e) => {
            eprintln!("[RDP] Failed to get display resolution: {}, using defaults", e);
            (1920, 1080)
        }
    };
    
    // 验证分辨率的合理性
    let (validated_width, validated_height) = validate_resolution(width, height);

    let response = ServerInfo {
        supports_webrtc: true,
        screen_width: validated_width,
        screen_height: validated_height,
        status: if server_status.healthy { "ok".to_string() } else { "degraded".to_string() },
        message: if server_status.display_available {
            "Server ready".to_string()
        } else {
            "Server ready (simulated display mode)".to_string()
        },
    };
    
    println!("[RDP] Server info response: {:?}", response);
    JsonResponse(response)
}

/// 异步检查服务器健康状态
async fn check_server_health() -> ServerStatus {
    // 检查显示是否可用
    let display_available = tokio::task::spawn_blocking(|| {
        use scrap::Display;
        Display::primary().is_ok()
    }).await.unwrap_or(false);
    
    ServerStatus {
        healthy: true,
        display_available,
        message: if display_available {
            "All systems operational".to_string()
        } else {
            "Running in simulated mode".to_string()
        },
    }
}

/// 异步获取显示器分辨率，带超时和回退机制
async fn get_display_resolution_with_fallback() -> anyhow::Result<(u32, u32)> {
    // 使用超时机制避免无限等待
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::task::spawn_blocking(|| get_display_resolution_safe())
    ).await;
    
    match result {
        Ok(Ok(resolution)) => Ok(resolution),
        Ok(Err(e)) => {
            eprintln!("[RDP] Display detection failed: {}", e);
            Ok((1920, 1080))
        }
        Err(_) => {
            eprintln!("[RDP] Display detection timed out, using defaults");
            Ok((1920, 1080))
        }
    }
}

/// 安全地获取显示器分辨率，在无显示器环境中返回默认值
fn get_display_resolution_safe() -> (u32, u32) {
    use scrap::Display;
    
    // 尝试获取主显示器，失败时使用默认分辨率
    match Display::primary() {
        Ok(display) => {
            let w = display.width() as u32;
            let h = display.height() as u32;
            
            // 验证获取的分辨率是否合理
            if w > 0 && h > 0 && w <= 8192 && h <= 8192 {
                println!("[RDP] Detected display resolution: {}x{}", w, h);
                (w, h)
            } else {
                println!("[RDP] Invalid display resolution detected ({}x{}), using default", w, h);
                (1920, 1080)
            }
        }
        Err(e) => {
            println!("[RDP] No display detected ({}), using default resolution 1920x1080", e);
            // 默认使用 1920x1080
            (1920, 1080)
        }
    }
}

/// 验证分辨率是否在合理范围内
fn validate_resolution(width: u32, height: u32) -> (u32, u32) {
    const MIN_RES: u32 = 320;
    const MAX_RES: u32 = 7680; // 8K
    
    let validated_width = width.clamp(MIN_RES, MAX_RES);
    let validated_height = height.clamp(MIN_RES, MAX_RES);
    
    // 确保是偶数（某些编码器的要求）
    let validated_width = validated_width & !1;
    let validated_height = validated_height & !1;
    
    if validated_width != width || validated_height != height {
        println!("[RDP] Resolution adjusted from {}x{} to {}x{}", width, height, validated_width, validated_height);
    }
    
    (validated_width, validated_height)
}

/// 创建信令路由
pub fn create_signaling_router(state: Arc<SignalingState>) -> Router {
    Router::new()
        .route("/api/rdp/info", get(get_server_info))
        .route("/api/rdp/session", post(create_session))
        .route("/api/rdp/offer", post(handle_offer))
        .route("/api/rdp/ice", post(handle_ice_candidate))
        .route("/api/rdp/ice-candidates", post(get_ice_candidates))
        .with_state(state)
}
