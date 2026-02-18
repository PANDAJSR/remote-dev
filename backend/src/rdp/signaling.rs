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
}

pub async fn get_server_info() -> JsonResponse<ServerInfo> {
    // 获取屏幕分辨率
    use scrap::Display;
    let (width, height) = match Display::primary() {
        Ok(display) => (display.width() as u32, display.height() as u32),
        Err(_) => (1920, 1080),
    };

    JsonResponse(ServerInfo {
        supports_webrtc: true,
        screen_width: width,
        screen_height: height,
    })
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
