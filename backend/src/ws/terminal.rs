use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
};
use futures::{sink::SinkExt, stream::StreamExt};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::models::websocket::WsMessage;

pub async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(socket: WebSocket) {
    println!("New WebSocket connection established");

    let pty_system = NativePtySystem::default();

    let pair = pty_system
        .openpty(PtySize {
            rows: 30,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("Failed to open PTY");

    let shell = if cfg!(windows) {
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
    cmd.env("TERM", "screen-256color");
    cmd.env("TERM_PROGRAM", "screen");
    cmd.env("TERM_PROGRAM_VERSION", "0.1.0");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("SHELL", &shell);

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

    if shell.contains("bash") {
        cmd.arg("-i");
        cmd.arg("-l");
        println!("Using bash with -i -l flags");
    }

    let _child = pair
        .slave
        .spawn_command(cmd)
        .expect("Failed to spawn shell");

    drop(pair.slave);

    let master = Arc::new(Mutex::new(pair.master));

    let master_for_reader = Arc::clone(&master);
    let mut reader = {
        let master = master_for_reader.lock().unwrap();
        master.try_clone_reader().expect("Failed to clone reader")
    };

    let master_for_writer = Arc::clone(&master);
    let mut writer = {
        let master = master_for_writer.lock().unwrap();
        master.take_writer().expect("Failed to take writer")
    };

    let (mut sender, mut receiver) = socket.split();

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

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

    let (write_tx, mut write_rx) = mpsc::unbounded_channel::<String>();
    let (resize_tx, mut resize_rx) = mpsc::unbounded_channel::<(u16, u16)>();

    std::thread::spawn(move || {
        loop {
            if let Ok(data) = write_rx.try_recv() {
                if writer.write_all(data.as_bytes()).is_err() {
                    break;
                }
                if writer.flush().is_err() {
                    break;
                }
            }

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

            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

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

    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    println!("WebSocket connection closed");
}
