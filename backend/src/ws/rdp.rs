use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
};
use futures::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

use crate::rdp::capture::create_frame_pipeline;
use crate::rdp::input::{InputController, InputEvent};

/// RDP WebSocket 消息
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum RdpWsMessage {
    #[serde(rename = "start")]
    Start { fps: u32 },
    #[serde(rename = "stop")]
    Stop,
    #[serde(rename = "input")]
    Input { data: InputEvent },
}

/// RDP 会话状态
struct RdpWsSession {
    capture_running: Arc<Mutex<bool>>,
    input_controller: Arc<InputController>,
}

impl RdpWsSession {
    fn new(screen_width: u32, screen_height: u32) -> anyhow::Result<Self> {
        let input_controller = Arc::new(InputController::new(screen_width, screen_height)?);
        
        Ok(Self {
            capture_running: Arc::new(Mutex::new(false)),
            input_controller,
        })
    }
}

pub async fn rdp_ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_rdp_socket)
}

async fn handle_rdp_socket(mut socket: WebSocket) {
    println!("New RDP WebSocket connection established");

    // 获取屏幕尺寸
    let (screen_width, screen_height) = {
        use scrap::Display;
        match Display::primary() {
            Ok(display) => (display.width() as u32, display.height() as u32),
            Err(_) => (1920, 1080),
        }
    };

    // 创建会话
    let session = match RdpWsSession::new(screen_width, screen_height) {
        Ok(s) => Arc::new(Mutex::new(s)),
        Err(e) => {
            eprintln!("Failed to create RDP session: {}", e);
            return;
        }
    };

    // 创建帧处理管道
    let (_capture, mut frame_receiver) = match create_frame_pipeline(30, 2000000) {
        Ok((c, r)) => (c, r),
        Err(e) => {
            eprintln!("Failed to create frame pipeline: {}", e);
            return;
        }
    };

    let (mut sender, mut receiver) = socket.split();
    
    // 帧发送任务
    let session_for_send = Arc::clone(&session);
    let send_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(frame_data) = frame_receiver.recv() => {
                    // 检查是否正在捕获
                    let running = *session_for_send.lock().await.capture_running.lock().await;
                    if running {
                        if sender.send(Message::Binary(frame_data)).await.is_err() {
                            break;
                        }
                    }
                }
                else => break,
            }
        }
    });

    // 消息接收任务
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    if let Ok(ws_msg) = serde_json::from_str::<RdpWsMessage>(&text) {
                        match ws_msg {
                            RdpWsMessage::Start { fps } => {
                                println!("Starting RDP capture at {} FPS", fps);
                                let mut session = session.lock().await;
                                *session.capture_running.lock().await = true;
                            }
                            RdpWsMessage::Stop => {
                                println!("Stopping RDP capture");
                                let mut session = session.lock().await;
                                *session.capture_running.lock().await = false;
                            }
                            RdpWsMessage::Input { data: event } => {
                                let session = session.lock().await;
                                if let Err(e) = session.input_controller.handle_event(event) {
                                    eprintln!("Input handling error: {}", e);
                                }
                            }
                        }
                    }
                }
                Message::Close(_) => {
                    println!("RDP WebSocket closed by client");
                    break;
                }
                _ => {}
            }
        }
    });

    // 等待任一任务结束
    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }

    println!("RDP WebSocket connection closed");
}
