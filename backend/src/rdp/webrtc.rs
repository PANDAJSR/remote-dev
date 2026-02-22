use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use serde::{Deserialize, Serialize};

use webrtc::{
    api::{
        interceptor_registry::register_default_interceptors,
        media_engine::MediaEngine,
        APIBuilder,
    },
    ice_transport::ice_server::RTCIceServer,
    interceptor::registry::Registry,
    peer_connection::{
        configuration::RTCConfiguration,
        peer_connection_state::RTCPeerConnectionState,
        sdp::session_description::RTCSessionDescription,
        RTCPeerConnection,
    },
    rtp_transceiver::rtp_codec::RTCRtpCodecCapability,
    track::track_local::{
        track_local_static_sample::TrackLocalStaticSample, TrackLocal,
    },
    media::Sample,
};
use bytes::Bytes;
use std::time::Duration;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;

use crate::rdp::capture::{ScreenCapture, Vp8Encoder};

/// SDP Offer/Answer 数据结构
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SdpOffer {
    pub sdp: String,
    pub session_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SdpAnswer {
    pub sdp: String,
    pub session_id: String,
    pub success: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IceCandidate {
    pub candidate: String,
    #[serde(rename = "sdpMid")]
    pub sdp_mid: Option<String>,
    #[serde(rename = "sdpMLineIndex")]
    pub sdp_mline_index: Option<u16>,
    pub session_id: String,
}

/// WebRTC 会话管理器
pub struct WebrtcManager {
    sessions: Arc<Mutex<std::collections::HashMap<String, Arc<RdpSession>>>>,
}

impl WebrtcManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub async fn create_session(
        &self,
        session_id: String,
    ) -> anyhow::Result<Arc<RdpSession>> {
        let session = Arc::new(RdpSession::new(session_id.clone()).await?);
        
        let mut sessions = self.sessions.lock().await;
        sessions.insert(session_id, Arc::clone(&session));
        
        Ok(session)
    }

    pub async fn get_session(&self, session_id: &str) -> Option<Arc<RdpSession>> {
        let sessions = self.sessions.lock().await;
        sessions.get(session_id).cloned()
    }
}

/// 单个 RDP 会话
pub struct RdpSession {
    pub id: String,
    pub peer_connection: Arc<RTCPeerConnection>,
    video_track: Arc<TrackLocalStaticSample>,
    ice_candidates: Arc<Mutex<Vec<IceCandidate>>>,
}

impl RdpSession {
    pub async fn new(session_id: String) -> anyhow::Result<Self> {
        // 创建 MediaEngine
        let mut m = MediaEngine::default();
        m.register_default_codecs()?;

        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut m)?;

        let api = APIBuilder::new()
            .with_media_engine(m)
            .with_interceptor_registry(registry)
            .build();

        // 配置 ICE 服务器
        let config = RTCConfiguration {
            ice_servers: vec![
                RTCIceServer {
                    urls: vec!["stun:stun.l.google.com:19302".to_owned()],
                    ..Default::default()
                },
                RTCIceServer {
                    urls: vec!["stun:stun1.l.google.com:19302".to_owned()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let peer_connection = Arc::new(api.new_peer_connection(config).await?);

        println!("[RDP] Creating H.264 video track...");
        let video_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: "video/H264".to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e028".to_owned(),
                rtcp_feedback: vec![],
            },
            "rdp-video".to_owned(),
            "remote-desktop".to_owned(),
        ));
        println!("[RDP] Video track created: codec=H264, profile-level-id=42e028");

        // 添加轨道到 PeerConnection
        let rtp_sender = peer_connection
            .add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await?;

        // 启动 RTCP 处理
        tokio::spawn(async move {
            let mut rtcp_buf = vec![0u8; 1500];
            while let Ok((_, _)) = rtp_sender.read(&mut rtcp_buf).await {
                // RTCP 处理
            }
        });

        let track_clone = Arc::clone(&video_track);
        let pc_clone_for_capture = Arc::clone(&peer_connection);
        let session_id_for_task = session_id.clone();

        // 启动屏幕捕获和发送
        tokio::spawn(async move {
            println!("[RDP] Starting capture task for session {}", session_id_for_task);
            if let Err(e) = Self::capture_and_send(track_clone, pc_clone_for_capture).await {
                eprintln!("[RDP] Capture error for session {}: {}", session_id_for_task, e);
            }
        });

        let ice_candidates = Arc::new(Mutex::new(Vec::new()));
        let ice_candidates_clone = Arc::clone(&ice_candidates);

        // 监听 ICE candidate
        peer_connection.on_ice_candidate(Box::new(move |candidate| {
            let candidates = Arc::clone(&ice_candidates_clone);
            Box::pin(async move {
                if let Some(c) = candidate {
                    if let Ok(json) = c.to_json() {
                        let ice = IceCandidate {
                            candidate: json.candidate,
                            sdp_mid: json.sdp_mid,
                            sdp_mline_index: json.sdp_mline_index.map(|x| x as u16),
                            session_id: String::new(),
                        };
                        let mut list = candidates.lock().await;
                        list.push(ice);
                    }
                }
            })
        }));

        // 监听连接状态
        let pc_clone = Arc::clone(&peer_connection);
        peer_connection.on_peer_connection_state_change(Box::new(move |state| {
            println!("[RDP] PeerConnection state: {:?}", state);
            if state == RTCPeerConnectionState::Failed {
                let pc = Arc::clone(&pc_clone);
                Box::pin(async move {
                    let _ = pc.close().await;
                })
            } else if state == RTCPeerConnectionState::Connected {
                println!("[RDP] Connection established, video streaming should start");
                Box::pin(async {})
            } else {
                Box::pin(async {})
            }
        }));

        Ok(Self {
            id: session_id,
            peer_connection,
            video_track,
            ice_candidates,
        })
    }

    /// 处理 SDP Offer，返回 Answer
    pub async fn handle_offer(&self, offer_sdp: String) -> anyhow::Result<String> {
        // 设置远程描述
        let offer = RTCSessionDescription::offer(offer_sdp)?;
        self.peer_connection.set_remote_description(offer).await?;

        // 创建 Answer
        let answer = self.peer_connection.create_answer(None).await?;
        
        // 设置本地描述
        self.peer_connection.set_local_description(answer.clone()).await?;

        // 等待 ICE 收集
        let mut gather_complete = self.peer_connection.gathering_complete_promise().await;
        let _ = gather_complete.recv().await;

        // 获取完整的 Answer
        let local_desc = self.peer_connection.local_description().await;
        let answer_sdp = local_desc.map(|d| d.sdp).unwrap_or_default();

        Ok(answer_sdp)
    }

    /// 添加 ICE candidate
    pub async fn add_ice_candidate(&self, candidate: IceCandidate) -> anyhow::Result<()> {
        let candidate_init = RTCIceCandidateInit {
            candidate: candidate.candidate,
            sdp_mid: candidate.sdp_mid,
            sdp_mline_index: candidate.sdp_mline_index.map(|x| x as u16),
            username_fragment: None,
        };

        self.peer_connection.add_ice_candidate(candidate_init).await?;
        Ok(())
    }

    /// 获取 ICE candidates
    pub async fn get_ice_candidates(&self) -> Vec<IceCandidate> {
        let candidates = self.ice_candidates.lock().await;
        candidates.clone()
    }

    /// 屏幕捕获和发送视频帧
    async fn capture_and_send(
        track: Arc<TrackLocalStaticSample>,
        peer_connection: Arc<RTCPeerConnection>,
    ) -> anyhow::Result<()> {
        println!("[WEBRTC] =========================================");
        println!("[WEBRTC] Starting capture_and_send pipeline...");
        println!("[WEBRTC] PeerConnection state at start: {:?}", peer_connection.connection_state());
        println!("[WEBRTC] ICE state at start: {:?}", peer_connection.ice_connection_state());

        // 初始化屏幕捕获
        let capture = match ScreenCapture::new() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[WEBRTC] FATAL: Failed to initialize screen capture: {}", e);
                return Err(e);
            }
        };

        let (width, height) = capture.dimensions();
        const TARGET_FPS: u32 = 30;
        println!("[WEBRTC] Screen capture initialized: {}x{} @ {} FPS", width, height, TARGET_FPS);
        println!("[WEBRTC] Track ID: remote-desktop");

        // 创建帧通道
        let (frame_tx, mut frame_rx) = mpsc::channel(60);

        // P1 Fix: 启动屏幕捕获（使用单例模式，支持多会话共享）
        capture.start_capture(frame_tx, TARGET_FPS).await;
        println!("[WEBRTC] Screen capture started (singleton mode)");

        // P1 Fix: 提高码率到 8Mbps 以获得更好的质量和帧率
        const TARGET_BITRATE: u32 = 8_000_000;
        println!("[WEBRTC] Initializing H.264 encoder (bitrate: {}Mbps)...", TARGET_BITRATE / 1_000_000);
        let mut encoder = match Vp8Encoder::new(width, height, TARGET_BITRATE) {
            Ok(enc) => {
                println!("[WEBRTC] H.264 encoder initialized successfully");
                enc
            }
            Err(e) => {
                eprintln!("[WEBRTC] FATAL: Failed to initialize H.264 encoder: {}", e);
                return Err(e);
            }
        };

        let mut frame_count = 0u64;
        let mut encoded_count = 0u64;
        let mut error_count = 0u64;
        let mut last_stats_time = std::time::Instant::now();
        let mut total_bytes_sent = 0u64;
        let mut no_connection_frames = 0u64;

        println!("[WEBRTC] Entering frame processing loop...");
        println!("[WEBRTC] Waiting for WebRTC connection...");
        
        // 处理捕获的帧并发送
        let mut timestamp = std::time::Instant::now();
        let start_time = std::time::Instant::now();
        
        while let Some(frame) = frame_rx.recv().await {
            frame_count += 1;

            let conn_state = peer_connection.connection_state();
            let ice_state = peer_connection.ice_connection_state();
            let signaling_state = peer_connection.signaling_state();
            
            let should_send = conn_state == RTCPeerConnectionState::Connected 
                || conn_state == RTCPeerConnectionState::Connecting
                || ice_state == webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Connected
                || ice_state == webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Completed
                || ice_state == webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Checking
                || (signaling_state == webrtc::peer_connection::signaling_state::RTCSignalingState::Stable 
                    && ice_state != webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::New
                    && ice_state != webrtc::ice_transport::ice_connection_state::RTCIceConnectionState::Failed);
                
            if !should_send {
                no_connection_frames += 1;
                if no_connection_frames % 60 == 1 {
                    println!("[WEBRTC] Waiting for WebRTC connection...");
                    println!("[WEBRTC]   Connection state: {:?}", conn_state);
                    println!("[WEBRTC]   ICE state: {:?}", ice_state);
                    println!("[WEBRTC]   Signaling state: {:?}", signaling_state);
                    println!("[WEBRTC]   Frames received while waiting: {}", no_connection_frames);
                }
                continue;
            } else if no_connection_frames > 0 {
                println!("[WEBRTC] Connection established! Resuming transmission.");
                println!("[WEBRTC]   Connection state: {:?}", conn_state);
                println!("[WEBRTC]   ICE state: {:?}", ice_state);
                no_connection_frames = 0;
            }

            if frame_count % 30 == 0 {
                println!("[WEBRTC] Pipeline stats: {} frames received, {} encoded, {} errors",
                    frame_count, encoded_count, error_count);
            }
            
            // 阶段3: 编码 (BGRA -> I420 -> H.264)
            println!("[WEBRTC] Stage 3: Encoding frame {}...", frame_count);
            match encoder.encode_frame(&frame) {
                Ok(encoded_data) => {
                    if !encoded_data.is_empty() {
                        encoded_count += 1;
                        total_bytes_sent += encoded_data.len() as u64;

                        // 分析H.264数据
                        let is_keyframe = encoded_data.len() > 4 && (
                            (encoded_data[4] & 0x1F) == 5 || // IDR slice
                            (encoded_data[4] & 0x1F) == 7 || // SPS
                            (encoded_data[4] & 0x1F) == 8    // PPS
                        );

                        if encoded_count <= 5 || encoded_count % 60 == 0 {
                            let preview: Vec<u8> = encoded_data.iter().take(16).cloned().collect();
                            println!("[WEBRTC] Encoded sample {}: {} bytes, keyframe: {}, nal_type: {:#04x}",
                                encoded_count, encoded_data.len(), is_keyframe,
                                if encoded_data.len() > 4 { encoded_data[4] } else { 0 });
                        }

                        // 阶段4: WebRTC传输
                        let sample = Sample {
                            data: Bytes::from(encoded_data),
                            duration: Duration::from_millis(33), // ~30fps
                            ..Default::default()
                        };

                        let write_result = track.write_sample(&sample).await;
                        match write_result {
                            Ok(_) => {
                                let should_log = encoded_count <= 5 || encoded_count % 30 == 0;
                                if should_log {
                                    println!("[WEBRTC] Sample written successfully #{} ({} bytes, is_keyframe: {})",
                                        encoded_count, sample.data.len(), is_keyframe);
                                }
                                
                                if is_keyframe {
                                    println!("[WEBRTC] KEYFRAME #{} SENT successfully! Video should start rendering now.",
                                        (encoded_count - 1) / 30 + 1);
                                }
                            }
                            Err(e) => {
                                eprintln!("[WEBRTC] FAILED to write sample #{}: {}", encoded_count, e);
                                let state = peer_connection.connection_state();
                                let ice_state = peer_connection.ice_connection_state();
                                eprintln!("[WEBRTC] Connection state: {:?}, ICE state: {:?}", state, ice_state);
                                
                                if state == RTCPeerConnectionState::Failed ||
                                   state == RTCPeerConnectionState::Closed {
                                    eprintln!("[WEBRTC] Connection lost (state: {:?}), stopping capture", state);
                                    break;
                                }
                            }
                        }

                        // 每5秒打印一次统计信息
                        let now = std::time::Instant::now();
                        if now.duration_since(last_stats_time).as_secs() >= 5 {
                            let elapsed_secs = now.duration_since(last_stats_time).as_secs_f64();
                            let fps = encoded_count as f64 / elapsed_secs.max(1.0);
                            let bitrate_kbps = (total_bytes_sent as f64 * 8.0 / elapsed_secs.max(1.0) / 1000.0) as u64;
                            let error_rate = if frame_count > 0 {
                                (error_count as f64 / frame_count as f64) * 100.0
                            } else { 0.0 };

                            println!("[WEBRTC] ===== 5-Second Statistics =====");
                            println!("[WEBRTC] FPS: {:.1} (target: {})", fps, TARGET_FPS);
                            println!("[WEBRTC] Bitrate: {} kbps", bitrate_kbps);
                            println!("[WEBRTC] Frames: {} received, {} encoded, {} errors ({:.1}% error rate)",
                                frame_count, encoded_count, error_count, error_rate);
                            println!("[WEBRTC] Total bytes sent: {} ({:.1} MB)",
                                total_bytes_sent, total_bytes_sent as f64 / 1_048_576.0);

                            if fps < TARGET_FPS as f64 * 0.7 {
                                println!("[WEBRTC] WARNING: FPS is below target! Check CPU usage and capture permissions.");
                            }
                            if error_rate > 10.0 {
                                println!("[WEBRTC] WARNING: High error rate detected! Check encoding configuration.");
                            }

                            last_stats_time = now;
                        }
                    } else {
                        println!("[WEBRTC] WARNING: Encoded data is EMPTY for frame {}", frame_count);
                        error_count += 1;
                    }
                }
                Err(e) => {
                    error_count += 1;
                    eprintln!("[WEBRTC] Stage 3 FAILED - Encoding error ({} total): {}", error_count, e);
                    continue;
                }
            }
        }

        // 最终统计
        let total_time = std::time::Instant::now();
        println!("[WEBRTC] =========================================");
        println!("[WEBRTC] Capture pipeline STOPPED");
        println!("[WEBRTC] Total frames received: {}", frame_count);
        println!("[WEBRTC] Total frames encoded: {}", encoded_count);
        println!("[WEBRTC] Total errors: {}", error_count);
        println!("[WEBRTC] Total bytes sent: {} ({:.2} MB)",
            total_bytes_sent, total_bytes_sent as f64 / 1_048_576.0);
        if frame_count > 0 {
            println!("[WEBRTC] Final error rate: {:.1}%", (error_count as f64 / frame_count as f64) * 100.0);
        }
        println!("[WEBRTC] =========================================");
        Ok(())
    }
}
