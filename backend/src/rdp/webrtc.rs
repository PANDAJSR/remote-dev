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
    frame_sender: mpsc::Sender<Vec<u8>>,
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

        // 创建 H.264 视频轨道
        let video_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: "video/H264".to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f".to_owned(),
                rtcp_feedback: vec![],
            },
            "rdp-video".to_owned(),
            "remote-desktop".to_owned(),
        ));

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

        // 创建帧发送通道
        let (frame_sender, mut frame_receiver) = mpsc::channel::<Vec<u8>>(100);
        let track_clone = Arc::clone(&video_track);

        // 启动屏幕捕获和发送
        tokio::spawn(async move {
            if let Err(e) = Self::capture_and_send(track_clone, &mut frame_receiver).await {
                eprintln!("Capture error: {}", e);
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
            println!("PeerConnection state: {:?}", state);
            if state == RTCPeerConnectionState::Failed {
                let pc = Arc::clone(&pc_clone);
                Box::pin(async move {
                    let _ = pc.close().await;
                })
            } else {
                Box::pin(async {})
            }
        }));

        Ok(Self {
            id: session_id,
            peer_connection,
            video_track,
            ice_candidates,
            frame_sender,
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
        _receiver: &mut mpsc::Receiver<Vec<u8>>,
    ) -> anyhow::Result<()> {
        println!("[CAPTURE] Starting capture_and_send...");
        
        // 初始化屏幕捕获
        let capture = match ScreenCapture::new() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[CAPTURE] Failed to initialize screen capture: {}", e);
                return Err(e);
            }
        };
        
        let (width, height) = capture.dimensions();
        println!("[CAPTURE] Screen capture initialized: {}x{} @ 30 FPS", width, height);
        
        // 创建帧通道
        let (frame_tx, mut frame_rx) = mpsc::channel(30);
        
        // 启动屏幕捕获 (30 FPS)
        capture.start_capture(frame_tx, 30);
        println!("[CAPTURE] Screen capture started");
        
        // 初始化 H.264 编码器 (2 Mbps bitrate)
        println!("[CAPTURE] Initializing H.264 encoder...");
        let mut encoder = match Vp8Encoder::new(width, height, 2_000_000) {
            Ok(enc) => {
                println!("[CAPTURE] H.264 encoder initialized successfully");
                enc
            }
            Err(e) => {
                eprintln!("[CAPTURE] Failed to initialize H.264 encoder: {}", e);
                return Err(e);
            }
        };
        
        let mut frame_count = 0u64;
        let mut encoded_count = 0u64;
        let mut error_count = 0u64;
        let mut last_fps_print = std::time::Instant::now();
        
        println!("[CAPTURE] Entering frame processing loop...");
        
        // 处理捕获的帧并发送
        while let Some(frame) = frame_rx.recv().await {
            frame_count += 1;
            
            if frame_count % 30 == 0 {
                println!("[CAPTURE] Received {} frames so far", frame_count);
            }
            
            // 编码帧: BGRA -> I420 -> H.264
            match encoder.encode_frame(&frame) {
                Ok(encoded_data) => {
                    if !encoded_data.is_empty() {
                        encoded_count += 1;
                        
                        // 创建 WebRTC 样本
                        let sample = Sample {
                            data: Bytes::from(encoded_data),
                            duration: Duration::from_millis(33), // ~30 FPS
                            ..Default::default()
                        };
                        
                        // 发送到 WebRTC
                        if let Err(e) = track.write_sample(&sample).await {
                            eprintln!("[CAPTURE] Failed to write sample to track: {}", e);
                            // 连接可能已断开，退出循环
                            break;
                        }
                        
                        if encoded_count % 30 == 0 {
                            println!("[CAPTURE] Sent {} encoded frames", encoded_count);
                        }
                        
                        // 每30秒打印一次 FPS
                        let now = std::time::Instant::now();
                        if now.duration_since(last_fps_print).as_secs() >= 30 {
                            let fps = encoded_count as f64 / now.duration_since(last_fps_print).as_secs_f64();
                            println!("[CAPTURE] Screen capture FPS: {:.1}, total frames: {}, errors: {}", 
                                fps, encoded_count, error_count);
                            last_fps_print = now;
                            encoded_count = 0;
                        }
                    } else {
                        println!("[CAPTURE] Encoded data is empty");
                    }
                }
                Err(e) => {
                    error_count += 1;
                    eprintln!("[CAPTURE] Frame encoding error ({} total): {}", error_count, e);
                    // 编码失败，跳过该帧，继续处理下一帧
                    continue;
                }
            }
        }
        
        println!("[CAPTURE] Screen capture stopped. Total frames received: {}, encoded: {}, errors: {}", 
            frame_count, encoded_count, error_count);
        Ok(())
    }
}
