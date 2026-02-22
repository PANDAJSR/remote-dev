use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
};
use futures::{sink::SinkExt, stream::StreamExt};
use portable_pty::{Child, CommandBuilder, NativePtySystem, PtySize, PtySystem};
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

    // 创建子进程并保存 Child 对象以便后续清理
    let child = pair
        .slave
        .spawn_command(cmd)
        .expect("Failed to spawn shell");

    // 将 child 包装在 Arc<Mutex> 中以便共享
    let child = Arc::new(Mutex::new(child));

    // 注意：Windows 上不要过早 drop slave，否则可能导致写入失败
    // 我们将在 cleanup 阶段统一处理
    #[cfg(not(windows))]
    drop(pair.slave);

    let master = Arc::new(Mutex::new(pair.master));

    let master_for_reader = Arc::clone(&master);
    let mut reader = {
        let master = master_for_reader.lock().unwrap();
        master.try_clone_reader().expect("Failed to clone reader")
    };

    let mut writer = {
        let master = master.lock().unwrap();
        master.take_writer().expect("Failed to take writer")
    };

    // 用于 resize 的 master clone
    let master_for_resize = Arc::clone(&master);

    let (mut sender, mut receiver) = socket.split();

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let shutdown_tx_reader = shutdown_tx.clone();
    let shutdown_tx_writer = shutdown_tx.clone();

    // 创建 reader 线程，使用更可靠的关闭机制
    let reader_handle = {
        let tx_clone = tx.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut shutdown_rx = shutdown_tx_reader.subscribe();
            loop {
                // 使用 try_recv 检查关闭信号，然后尝试非阻塞读取
                if shutdown_rx.try_recv().is_ok() {
                    println!("Reader thread received shutdown signal");
                    break;
                }
                
                // 尝试读取数据，使用较小的超时
                match reader.read(&mut buf) {
                    Ok(0) => {
                        // EOF - writer 已关闭，退出
                        println!("PTY reader received EOF, exiting");
                        break;
                    }
                    Ok(n) => {
                        let data = String::from_utf8_lossy(&buf[..n]).to_string();
                        if tx_clone.send(data).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        if e.kind() != std::io::ErrorKind::Interrupted {
                            eprintln!("PTY read error: {}", e);
                            break;
                        }
                    }
                }
            }
            println!("PTY reader thread exited");
        })
    };

    let (write_tx, mut write_rx) = mpsc::unbounded_channel::<String>();
    let (resize_tx, mut resize_rx) = mpsc::unbounded_channel::<(u16, u16)>();

    // 创建 writer 线程
    let writer_handle = std::thread::spawn(move || {
        let mut shutdown_rx = shutdown_tx_writer.subscribe();
        loop {
            // 检查是否需要关闭
            if shutdown_rx.try_recv().is_ok() {
                println!("Writer thread received shutdown signal");
                break;
            }

            if let Ok(data) = write_rx.try_recv() {
                if writer.write_all(data.as_bytes()).is_err() {
                    break;
                }
                if writer.flush().is_err() {
                    break;
                }
            }

            if let Ok((cols, rows)) = resize_rx.try_recv() {
                let master = master_for_resize.lock().unwrap();
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
        // 重要：关闭 writer，让 reader 收到 EOF
        drop(writer);
        println!("PTY writer thread exited");
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

    // 等待任一任务完成（连接关闭）
    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }

    println!("WebSocket connection closing, cleaning up terminal...");

    // ========== 正确的清理顺序 ==========
    // 这是修复 conhost.exe 泄漏的关键！
    
    // 1. 首先关闭 WebSocket 端的发送通道
    drop(tx);

    // 2. 发送关闭信号给读写线程
    let _ = shutdown_tx.send(());

    // 3. 等待 writer 线程退出（它会 drop writer，让 reader 收到 EOF）
    tokio::task::spawn_blocking(move || {
        let timeout = std::time::Duration::from_secs(2);
        let start = std::time::Instant::now();
        
        while start.elapsed() < timeout {
            if writer_handle.is_finished() {
                let _ = writer_handle.join();
                println!("Writer thread joined successfully");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }).await.ok();

    // 4. 等待 reader 线程退出（writer 关闭后，reader 会收到 EOF）
    tokio::task::spawn_blocking(move || {
        let timeout = std::time::Duration::from_secs(2);
        let start = std::time::Instant::now();
        
        while start.elapsed() < timeout {
            if reader_handle.is_finished() {
                let _ = reader_handle.join();
                println!("Reader thread joined successfully");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }).await.ok();

    // 5. 获取 master 的所有权并关闭它（关键步骤！）
    // 由于 reader 和 writer 线程已经退出，Arc 的引用计数应该是 1
    println!("Closing PTY master...");
    match Arc::try_unwrap(master) {
        Ok(mutex) => {
            match mutex.into_inner() {
                Ok(master) => {
                    drop(master);
                    println!("PTY master closed successfully");
                }
                Err(e) => {
                    eprintln!("Failed to unwrap Mutex: {:?}", e);
                }
            }
        }
        Err(arc) => {
            // 如果还有其他人持有 Arc，我们无法获取所有权
            // 但这不应该发生，因为我们已经等待了 reader 和 writer
            eprintln!("Warning: Cannot unwrap master Arc, {} references remain", Arc::strong_count(&arc));
            // 让 Arc 在作用域结束时自动 drop
        }
    }

    // 6. 关闭子进程
    {
        let mut child = child.lock().unwrap();
        println!("Killing child process...");
        if let Err(e) = kill_child_process(&mut **child) {
            eprintln!("Failed to kill child process: {}", e);
        }
    }

    // 7. 在 Windows 上，最后才 drop slave 句柄
    #[cfg(windows)]
    {
        println!("Dropping slave handle on Windows...");
        drop(pair.slave);
    }

    println!("WebSocket connection closed and terminal cleaned up");
}

/// 终止子进程
fn kill_child_process(child: &mut dyn Child) -> Result<(), Box<dyn std::error::Error>> {
    // 首先尝试检查进程是否已经结束
    match child.try_wait()? {
        Some(_exit_status) => {
            // 进程已经结束
            println!("Child process already exited");
            return Ok(());
        }
        None => {
            // 进程仍在运行，需要终止
        }
    }

    // 获取进程 ID
    let pid = child.process_id();
    println!("Killing child process with PID: {:?}", pid);
    
    // 使用 ChildKiller trait 的 kill 方法
    if let Err(e) = child.kill() {
        eprintln!("Failed to kill child process using ChildKiller: {}", e);
        
        // 备用方案：使用系统命令强制终止
        #[cfg(windows)]
        if let Some(pid) = pid {
            use std::process::Command;
            println!("Falling back to taskkill for PID: {}", pid);
            
            let output = Command::new("taskkill")
                .args(&["/F", "/T", "/PID", &pid.to_string()])
                .output()?;
            
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                eprintln!("taskkill failed: {}", stderr);
            }
        }
        
        #[cfg(unix)]
        if let Some(pid) = pid {
            use libc::{kill, SIGTERM, SIGKILL};
            let pid_i32 = pid as i32;
            println!("Falling back to SIGKILL for PID: {}", pid);
            
            unsafe {
                // 先尝试 SIGTERM
                if kill(pid_i32, SIGTERM) != 0 {
                    // SIGTERM 失败，尝试 SIGKILL
                    if kill(pid_i32, SIGKILL) != 0 {
                        return Err("Failed to kill process with SIGKILL".into());
                    }
                }
            }
        }
    } else {
        println!("Successfully sent kill signal to child process");
    }

    // 等待进程完全退出
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(5) {
        match child.try_wait()? {
            Some(_) => {
                println!("Child process confirmed exited");
                return Ok(());
            }
            None => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }

    Err("Timeout waiting for child process to exit".into())
}
